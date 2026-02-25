//! Bridge between the MCP server and the running synth engine.
//!
//! `AppSynthBridge` implements `SynthBridge` by reading from `EngineState`
//! (shared graph, meters, transport) and sending commands via `CommandSender`.

use std::sync::Arc;

use synth_core::{MidiNote, Param, PortDirection, Velocity};
use synth_engine::EngineCommand;
use synth_engine::commands::ModuleId;
use synth_engine::instrument::{InstrumentId, MidiChannel};
use synth_mcp::bridge::SynthBridge;
use synth_mcp::bridge::{
    BridgeNoteData, BridgeNoteUpdate, BridgePatternData, BridgePlacementData, BridgeSongPlacement,
    BridgeTrackData,
};
use synth_mcp::error::McpBridgeError;
use synth_mcp::types::{
    BatchItemResult, BatchResult, ConnectionInfo, DiagnosticSeverity, EngineStatus,
    ExamplePatchInfo, GraphDiagnostic, InstrumentInfo, ModuleInfo, ModuleTypeInfo, NoteInfo,
    ParameterInfo, PatternInfo, PlacementInfo, SetSongResult, SongInfo, TrackInfo,
    UiConnectionInfo, UiModuleInfo, UiOverlap, UiSnapshot,
};

use crate::mcp_shared::McpSharedState;
use crate::session::SynthSession;

/// Bridge implementation for the modular_synth application.
pub struct AppSynthBridge {
    session: Arc<SynthSession>,
    shared: Arc<McpSharedState>,
}

impl AppSynthBridge {
    /// Create a new bridge with access to the session and shared MCP state.
    pub fn new(session: Arc<SynthSession>, shared: Arc<McpSharedState>) -> Self {
        Self { session, shared }
    }
}

impl AppSynthBridge {
    /// Validate that a module exists and has the given port in the expected direction.
    fn validate_port(
        &self,
        instrument_id: InstrumentId,
        module_str: &str,
        port: &str,
        direction: PortDirection,
    ) -> Result<(), McpBridgeError> {
        let mid: ModuleId = module_str
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_str.to_string()))?;

        let descriptor = self
            .session
            .module_descriptor(instrument_id, mid)
            .ok_or_else(|| McpBridgeError::ModuleNotFound(module_str.to_string()))?;

        let has_port = descriptor
            .ports
            .iter()
            .any(|p| p.name == port && p.direction == direction);

        if !has_port {
            return Err(McpBridgeError::PortNotFound {
                module: module_str.to_string(),
                port: port.to_string(),
            });
        }

        Ok(())
    }

    /// Validate that an instrument exists in the shared snapshots.
    fn validate_instrument(&self, instrument_id: u64) -> Result<(), McpBridgeError> {
        if !self
            .session
            .instrument_exists(InstrumentId::new(instrument_id))
        {
            return Err(McpBridgeError::InstrumentNotFound(instrument_id));
        }
        Ok(())
    }

    /// Convert an `InstrumentSnapshot` to an `InstrumentInfo`.
    fn snapshot_to_info(snap: &synth_engine::shared_state::InstrumentSnapshot) -> InstrumentInfo {
        InstrumentInfo {
            id: snap.id.as_u64(),
            name: snap.name.clone(),
            midi_channel: snap.midi_channel,
            volume: snap.volume,
            pan: snap.pan,
            enabled: snap.enabled,
            muted: snap.muted,
            solo: snap.solo,
            module_count: snap.module_count,
            effect_count: snap.effect_count,
        }
    }
}

impl SynthBridge for AppSynthBridge {
    fn list_instruments(&self) -> Result<Vec<InstrumentInfo>, McpBridgeError> {
        let snapshots = self.session.list_instruments();
        Ok(snapshots.iter().map(Self::snapshot_to_info).collect())
    }

    fn get_instrument_info(&self, instrument_id: u64) -> Result<InstrumentInfo, McpBridgeError> {
        let snapshots = self.session.list_instruments();
        snapshots
            .iter()
            .find(|s| s.id.as_u64() == instrument_id)
            .map(Self::snapshot_to_info)
            .ok_or(McpBridgeError::InstrumentNotFound(instrument_id))
    }

    fn list_modules(&self, instrument_id: u64) -> Result<Vec<ModuleInfo>, McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let state = self.session.state();
        let inst_id = InstrumentId::new(instrument_id);
        let modules = state.shared_graph.get_modules_for_instrument(inst_id);
        let connections = state.shared_graph.get_connections_for_instrument(inst_id);

        Ok(modules
            .into_iter()
            .map(|m| {
                let id_str = m.id.to_string();
                let input_ports: Vec<String> = m.input_connection_counts.keys().cloned().collect();
                let output_ports: Vec<String> =
                    m.output_connection_counts.keys().cloned().collect();

                // Build input/output ports from connections if snapshot doesn't have them
                let mut inputs = input_ports;
                let mut outputs = output_ports;

                for conn in &connections {
                    if conn.to_module.to_string() == id_str && !inputs.contains(&conn.to_port) {
                        inputs.push(conn.to_port.clone());
                    }
                    if conn.from_module.to_string() == id_str && !outputs.contains(&conn.from_port)
                    {
                        outputs.push(conn.from_port.clone());
                    }
                }

                ModuleInfo {
                    id: id_str,
                    module_type: m.module_type.name().to_string(),
                    name: m.name,
                    bypassed: m.bypass_state == synth_core::BypassState::Bypassed,
                    parameters: m
                        .parameters
                        .iter()
                        .map(|p| ParameterInfo {
                            name: p.name().to_string(),
                            value: p.as_f32(),
                            display: format_param_display(p),
                        })
                        .collect(),
                    input_ports: inputs,
                    output_ports: outputs,
                }
            })
            .collect())
    }

    fn get_module_info(
        &self,
        instrument_id: u64,
        module_id: &str,
    ) -> Result<ModuleInfo, McpBridgeError> {
        let modules = self.list_modules(instrument_id)?;
        modules
            .into_iter()
            .find(|m| m.id == module_id)
            .ok_or_else(|| McpBridgeError::ModuleNotFound(module_id.to_string()))
    }

    fn get_connections(&self, instrument_id: u64) -> Result<Vec<ConnectionInfo>, McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let connections = self
            .session
            .state()
            .shared_graph
            .get_connections_for_instrument(InstrumentId::new(instrument_id));
        Ok(connections
            .into_iter()
            .map(|c| ConnectionInfo {
                from_module: c.from_module.to_string(),
                from_port: c.from_port,
                to_module: c.to_module.to_string(),
                to_port: c.to_port,
            })
            .collect())
    }

    fn get_parameter(
        &self,
        instrument_id: u64,
        module_id: &str,
        param_name: &str,
    ) -> Result<ParameterInfo, McpBridgeError> {
        let module = self.get_module_info(instrument_id, module_id)?;
        module
            .parameters
            .into_iter()
            .find(|p| p.name == param_name)
            .ok_or_else(|| McpBridgeError::ParameterNotFound(param_name.to_string()))
    }

    fn get_engine_status(&self) -> Result<EngineStatus, McpBridgeError> {
        let state = self.session.state();
        let (peak_left, peak_right) = state.meters.get_peak();
        let (rms_left, rms_right) = state.meters.get_rms();

        Ok(EngineStatus {
            cpu_usage: state.cpu_usage.load(),
            voice_count: state.voice_count.load(),
            sample_rate: state.sample_rate.load(),
            peak_left,
            peak_right,
            rms_left,
            rms_right,
            master_volume: state.master_volume.load(),
            tempo: state.transport.get_tempo(),
            is_playing: state.transport.is_playing(),
            instrument_count: state.instrument_snapshots.read().len(),
        })
    }

    fn get_graph_diagnostics(
        &self,
        instrument_id: u64,
    ) -> Result<Vec<GraphDiagnostic>, McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let mut diagnostics = Vec::new();
        let state = self.session.state();
        let inst_id = InstrumentId::new(instrument_id);
        let modules = state.shared_graph.get_modules_for_instrument(inst_id);
        let connections = state.shared_graph.get_connections_for_instrument(inst_id);

        if modules.is_empty() {
            diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Info,
                module_id: None,
                message: "Voice graph is empty — no modules added yet".to_string(),
            });
            return Ok(diagnostics);
        }

        // Check for disconnected modules
        for module in &modules {
            let id_str = module.id.to_string();
            let has_input = connections
                .iter()
                .any(|c| c.to_module.to_string() == id_str);
            let has_output = connections
                .iter()
                .any(|c| c.from_module.to_string() == id_str);

            // ModMatrix and KineticModulator work via parameters, not cables
            let works_via_params = matches!(
                module.id.module_type,
                synth_core::ModuleType::ModMatrix | synth_core::ModuleType::KineticModulator
            );

            if !has_input && !has_output && !works_via_params {
                diagnostics.push(GraphDiagnostic {
                    severity: DiagnosticSeverity::Warning,
                    module_id: Some(id_str.clone()),
                    message: format!("Module {} ({}) has no connections", id_str, module.name),
                });
            } else if !has_output
                && module.id.module_type != synth_core::ModuleType::StereoOutput
                && module.id.module_type != synth_core::ModuleType::Amplifier
            {
                diagnostics.push(GraphDiagnostic {
                    severity: DiagnosticSeverity::Warning,
                    module_id: Some(id_str.clone()),
                    message: format!(
                        "Module {} ({}) has inputs but no outputs — signal dead-end",
                        id_str, module.name
                    ),
                });
            }
        }

        if diagnostics.is_empty() {
            diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Info,
                module_id: None,
                message: format!(
                    "Graph looks healthy: {} modules, {} connections",
                    modules.len(),
                    connections.len()
                ),
            });
        }

        Ok(diagnostics)
    }

    // === Instrument lifecycle ===

    fn create_instrument(&self, name: &str) -> Result<InstrumentInfo, McpBridgeError> {
        let id = self
            .session
            .add_instrument(name)
            .map_err(|_| McpBridgeError::CommandSendFailed)?;

        // Return basic info — the engine will update the snapshots asynchronously
        Ok(InstrumentInfo {
            id: id.as_u64(),
            name: name.to_string(),
            midi_channel: 1,
            volume: 1.0,
            pan: 0.0,
            enabled: true,
            muted: false,
            solo: false,
            module_count: 0,
            effect_count: 0,
        })
    }

    fn delete_instrument(&self, instrument_id: u64) -> Result<(), McpBridgeError> {
        if instrument_id == 0 {
            return Err(McpBridgeError::Other(
                "cannot delete the default instrument".to_string(),
            ));
        }
        self.validate_instrument(instrument_id)?;
        self.session
            .remove_instrument(InstrumentId::new(instrument_id))
            .map_err(|_| McpBridgeError::CommandSendFailed)
    }

    fn rename_instrument(&self, instrument_id: u64, name: &str) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .rename_instrument(InstrumentId::new(instrument_id), name)
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn set_instrument_volume(&self, instrument_id: u64, volume: f32) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_volume(InstrumentId::new(instrument_id), volume)
            .map_err(|_| McpBridgeError::CommandSendFailed)
    }

    fn set_instrument_pan(&self, instrument_id: u64, pan: f32) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_pan(InstrumentId::new(instrument_id), pan)
            .map_err(|_| McpBridgeError::CommandSendFailed)
    }

    fn set_instrument_mute(&self, instrument_id: u64, muted: bool) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_mute(InstrumentId::new(instrument_id), muted)
            .map_err(|_| McpBridgeError::CommandSendFailed)
    }

    fn set_instrument_solo(&self, instrument_id: u64, solo: bool) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_solo(InstrumentId::new(instrument_id), solo)
            .map_err(|_| McpBridgeError::CommandSendFailed)
    }

    fn set_instrument_midi_channel(
        &self,
        instrument_id: u64,
        channel: u8,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let midi_channel = MidiChannel::from_one_indexed(channel).unwrap_or(MidiChannel::CH1);
        self.session
            .set_instrument_midi_channel(InstrumentId::new(instrument_id), midi_channel)
            .map_err(|_| McpBridgeError::CommandSendFailed)
    }

    fn set_instrument_enabled(
        &self,
        instrument_id: u64,
        enabled: bool,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_enabled(InstrumentId::new(instrument_id), enabled)
            .map_err(|_| McpBridgeError::CommandSendFailed)
    }

    fn set_parameter(
        &self,
        instrument_id: u64,
        module_id: &str,
        param_name: &str,
        value: f32,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        // Find the module and its current parameter to construct the correct Param variant
        let module_snapshot = self
            .session
            .state()
            .shared_graph
            .get_modules_for_instrument(InstrumentId::new(instrument_id))
            .into_iter()
            .find(|m| m.id.to_string() == module_id)
            .ok_or_else(|| McpBridgeError::ModuleNotFound(module_id.to_string()))?;

        // Find matching param and create a new one with updated value
        let param = module_snapshot
            .parameters
            .iter()
            .find(|p| p.name() == param_name)
            .ok_or_else(|| McpBridgeError::ParameterNotFound(param_name.to_string()))?;

        let new_param = param.with_f32(value);

        if self
            .session
            .command_sender()
            .send(EngineCommand::SetModuleParameter {
                instrument_id: Some(InstrumentId::new(instrument_id)),
                module_id: module_snapshot.id,
                param: new_param,
            })
        {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed)
        }
    }

    fn note_on(&self, note: u8, velocity: u8, channel: u8) -> Result<(), McpBridgeError> {
        let midi_channel = MidiChannel::from_one_indexed(channel).unwrap_or(MidiChannel::CH1);
        if self.session.command_sender().send(EngineCommand::NoteOn {
            note: MidiNote::new(note),
            velocity: Velocity::from_midi(velocity),
            channel: midi_channel,
        }) {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed)
        }
    }

    fn note_off(&self, note: u8, channel: u8) -> Result<(), McpBridgeError> {
        let midi_channel = MidiChannel::from_one_indexed(channel).unwrap_or(MidiChannel::CH1);
        if self.session.command_sender().send(EngineCommand::NoteOff {
            note: MidiNote::new(note),
            channel: midi_channel,
        }) {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed)
        }
    }

    fn list_example_patches(&self) -> Result<Vec<ExamplePatchInfo>, McpBridgeError> {
        let categories = crate::patches::categorized_patches();
        let mut result = Vec::new();
        for (category, patches) in categories {
            for patch in patches {
                result.push(ExamplePatchInfo {
                    name: patch.name.clone(),
                    category: category.to_string(),
                    description: patch.description.clone().unwrap_or_default(),
                    tags: patch.tags.clone(),
                    module_count: patch.modules.len(),
                    connection_count: patch.connections.len(),
                });
            }
        }
        Ok(result)
    }

    fn load_example_patch(&self, name: &str) -> Result<String, McpBridgeError> {
        let categories = crate::patches::categorized_patches();
        let name_lower = name.to_ascii_lowercase();

        for (_category, patches) in categories {
            for patch in patches {
                if patch.name.to_ascii_lowercase() == name_lower {
                    let patch_name = patch.name.clone();
                    if let Ok(mut pending) = self.shared.pending_patch.lock() {
                        *pending = Some((patch, patch_name.clone()));
                    }
                    return Ok(format!("OK: {patch_name} queued for loading"));
                }
            }
        }

        Err(McpBridgeError::PatchNotFound(name.to_string()))
    }

    fn get_ui_snapshot(&self, instrument_id: u64) -> Result<UiSnapshot, McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let layout = self
            .shared
            .ui_layout
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        let modules: Vec<UiModuleInfo> = layout
            .modules
            .iter()
            .map(|m| UiModuleInfo {
                id: m.id.clone(),
                module_type: m.module_type.clone(),
                name: m.name.clone(),
                position: m.position,
                size: m.size,
                parameters: m.parameters.clone(),
            })
            .collect();

        let connections: Vec<UiConnectionInfo> = layout
            .connections
            .iter()
            .map(|c| UiConnectionInfo {
                from_module: c.from_module.clone(),
                from_port: c.from_port.clone(),
                to_module: c.to_module.clone(),
                to_port: c.to_port.clone(),
            })
            .collect();

        // Compute overlaps between module pairs
        let overlaps = compute_overlaps(&layout.modules);

        Ok(UiSnapshot {
            patch_name: layout.patch_name,
            modules,
            connections,
            window_size: layout.window_size,
            overlaps,
        })
    }

    fn list_module_types(&self) -> Result<Vec<ModuleTypeInfo>, McpBridgeError> {
        use crate::module_factory::{ALL_MODULE_TYPES, get_descriptor};
        use synth_core::PortDirection;

        let mut result = Vec::new();
        for &mt in ALL_MODULE_TYPES {
            if let Some(desc) = get_descriptor(mt) {
                let category = if mt.is_voice_module() {
                    "voice"
                } else if mt.is_effect() {
                    "effect"
                } else {
                    "visualizer"
                };

                let input_ports = desc
                    .ports
                    .iter()
                    .filter(|p| p.direction == PortDirection::Input)
                    .map(|p| p.name.clone())
                    .collect();
                let output_ports = desc
                    .ports
                    .iter()
                    .filter(|p| p.direction == PortDirection::Output)
                    .map(|p| p.name.clone())
                    .collect();
                let parameters = desc.parameters.iter().map(|p| p.name.clone()).collect();

                result.push(ModuleTypeInfo {
                    type_key: mt.prefix().to_string(),
                    name: mt.name().to_string(),
                    category: category.to_string(),
                    input_ports,
                    output_ports,
                    parameters,
                });
            }
        }
        Ok(result)
    }

    fn add_module(&self, instrument_id: u64, module_type: &str) -> Result<String, McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let mt = synth_core::ModuleType::from_prefix(module_type)
            .ok_or_else(|| McpBridgeError::InvalidModuleType(module_type.to_string()))?;

        let (module_id, _descriptor) = self
            .session
            .add_module(InstrumentId::new(instrument_id), mt)
            .map_err(|e| McpBridgeError::InvalidModuleType(e.to_string()))?;

        Ok(format!("OK: {} added as {}", mt.name(), module_id))
    }

    fn remove_module(&self, instrument_id: u64, module_id: &str) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;

        self.session
            .remove_module(InstrumentId::new(instrument_id), mid)
            .map_err(|e| McpBridgeError::ModuleNotFound(e.to_string()))
    }

    fn connect(
        &self,
        instrument_id: u64,
        from_module: &str,
        from_port: &str,
        to_module: &str,
        to_port: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let inst_id = InstrumentId::new(instrument_id);

        self.validate_port(inst_id, from_module, from_port, PortDirection::Output)?;
        self.validate_port(inst_id, to_module, to_port, PortDirection::Input)?;

        let from_id: ModuleId = from_module
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(from_module.to_string()))?;
        let to_id: ModuleId = to_module
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(to_module.to_string()))?;

        self.session
            .connect(
                inst_id,
                from_id,
                from_port.to_string(),
                to_id,
                to_port.to_string(),
            )
            .map_err(|_| McpBridgeError::CommandSendFailed)
    }

    fn disconnect(
        &self,
        instrument_id: u64,
        from_module: &str,
        from_port: &str,
        to_module: &str,
        to_port: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let inst_id = InstrumentId::new(instrument_id);

        self.validate_port(inst_id, from_module, from_port, PortDirection::Output)?;
        self.validate_port(inst_id, to_module, to_port, PortDirection::Input)?;

        let from_id: ModuleId = from_module
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(from_module.to_string()))?;
        let to_id: ModuleId = to_module
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(to_module.to_string()))?;

        self.session
            .disconnect(
                InstrumentId::new(instrument_id),
                from_id,
                from_port.to_string(),
                to_id,
                to_port.to_string(),
            )
            .map_err(|_| McpBridgeError::CommandSendFailed)
    }

    fn clear_graph(&self, instrument_id: u64) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        self.session
            .clear_graph(InstrumentId::new(instrument_id))
            .map_err(|_| McpBridgeError::CommandSendFailed)
    }

    // === Sequencer: Song ===

    fn get_song_info(&self) -> Result<SongInfo, McpBridgeError> {
        let song = self
            .shared
            .song
            .read()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let ts = song.default_time_signature;
        Ok(SongInfo {
            name: song.name.clone(),
            author: song.author.clone(),
            tempo: song.default_tempo.0,
            time_signature: format!("{}/{}", ts.numerator, ts.denominator),
            length_seconds: song.length_seconds(),
            pattern_count: song.pattern_count(),
            track_count: song.track_count(),
        })
    }

    fn set_song_tempo(&self, bpm: f32) -> Result<(), McpBridgeError> {
        {
            let mut song = self
                .shared
                .song
                .write()
                .map_err(|_| McpBridgeError::SongLockPoisoned)?;
            song.default_tempo = synth_core::Bpm::new(bpm);
        }
        // Also update engine transport tempo
        let _ = self
            .session
            .command_sender()
            .send(EngineCommand::SetTempo(synth_core::Bpm::new(bpm)));
        Ok(())
    }

    fn set_song_name(&self, name: &str) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        song.name = name.to_string();
        Ok(())
    }

    // === Sequencer: Patterns ===

    fn list_patterns(&self) -> Result<Vec<PatternInfo>, McpBridgeError> {
        let song = self
            .shared
            .song
            .read()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let mut patterns: Vec<PatternInfo> = song
            .patterns()
            .map(|p| PatternInfo {
                id: p.id.0,
                name: p.name.clone(),
                length_beats: ticks_to_beats(p.length.0),
                note_count: p.note_count(),
            })
            .collect();
        patterns.sort_by_key(|p| p.id);
        Ok(patterns)
    }

    fn create_pattern(&self, name: &str, length_beats: f32) -> Result<u32, McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let duration = synth_sequencer::Duration(beats_to_ticks(length_beats));
        let id = song.create_pattern(duration);
        if let Some(pattern) = song.pattern_mut(id) {
            pattern.name = name.to_string();
        }
        Ok(id.0)
    }

    fn delete_pattern(&self, pattern_id: u32) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let id = synth_sequencer::PatternId::new(pattern_id);
        song.delete_pattern(id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        Ok(())
    }

    // === Sequencer: Notes ===

    fn list_notes(&self, pattern_id: u32) -> Result<Vec<NoteInfo>, McpBridgeError> {
        let song = self
            .shared
            .song
            .read()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let id = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern(id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        Ok(pattern
            .notes()
            .iter()
            .map(|n| NoteInfo {
                id: n.id.0,
                pitch: n.pitch.as_midi(),
                pitch_name: n.pitch.to_string(),
                start_beat: ticks_to_beats(n.start.0),
                duration_beats: n.duration.map_or(1.0, |d| ticks_to_beats(d.0)),
                velocity: synth_core::Velocity::to_midi(n.velocity),
            })
            .collect())
    }

    fn add_note(
        &self,
        pattern_id: u32,
        pitch: u8,
        start_beat: f32,
        duration_beats: f32,
        velocity: u8,
    ) -> Result<u64, McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let p = synth_sequencer::Pitch::new(pitch).unwrap_or(synth_sequencer::Pitch::MIDDLE_C);
        let start = synth_sequencer::PatternTick(beats_to_ticks(start_beat));
        let vel = synth_core::Velocity::from_midi(velocity);
        let instrument = synth_sequencer::SeqInstrumentId(0);

        let note = synth_sequencer::Note::new(
            synth_sequencer::NoteId(0), // will be reassigned by insert_note
            start,
            p,
            vel,
            instrument,
        )
        .with_duration(synth_sequencer::Duration(beats_to_ticks(duration_beats)));

        let note_id = pattern.insert_note(note);
        Ok(note_id.0)
    }

    fn remove_note(&self, pattern_id: u32, note_id: u64) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        let nid = synth_sequencer::NoteId(note_id);
        pattern
            .remove_note(nid)
            .ok_or(McpBridgeError::NoteNotFound(note_id))?;
        Ok(())
    }

    fn update_note(
        &self,
        pattern_id: u32,
        note_id: u64,
        pitch: Option<u8>,
        start_beat: Option<f32>,
        duration_beats: Option<f32>,
        velocity: Option<u8>,
    ) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        let nid = synth_sequencer::NoteId(note_id);
        let note = pattern
            .note_mut(nid)
            .ok_or(McpBridgeError::NoteNotFound(note_id))?;

        if let Some(p) = pitch
            && let Some(new_pitch) = synth_sequencer::Pitch::new(p)
        {
            note.pitch = new_pitch;
        }
        if let Some(s) = start_beat {
            note.start = synth_sequencer::PatternTick(beats_to_ticks(s));
        }
        if let Some(d) = duration_beats {
            note.duration = Some(synth_sequencer::Duration(beats_to_ticks(d)));
        }
        if let Some(v) = velocity {
            note.velocity = synth_core::Velocity::from_midi(v);
        }
        Ok(())
    }

    // === Sequencer: Tracks ===

    fn list_tracks(&self) -> Result<Vec<TrackInfo>, McpBridgeError> {
        let song = self
            .shared
            .song
            .read()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let mut tracks: Vec<TrackInfo> = song
            .tracks()
            .map(|t| TrackInfo {
                id: t.id.0,
                name: t.name.clone(),
                instrument_id: t.instrument.map(|i| i.0),
                volume: t.volume.as_f32(),
                mute: t.mute,
                solo: t.solo,
            })
            .collect();
        tracks.sort_by_key(|t| t.id);
        Ok(tracks)
    }

    fn create_track(&self, name: &str, instrument_id: Option<u16>) -> Result<u16, McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let id = song.create_track(name);
        if let Some(inst_id) = instrument_id
            && let Some(track) = song.track_mut(id)
        {
            track.instrument = Some(synth_sequencer::SeqInstrumentId(inst_id));
        }
        Ok(id.0)
    }

    // === Sequencer: Arrangement ===

    fn place_pattern(
        &self,
        pattern_id: u32,
        track_id: u16,
        start_beat: f32,
    ) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let tid = synth_sequencer::TrackId(track_id);
        let tick = synth_sequencer::Tick(u64::from(beats_to_ticks(start_beat)));

        // Validate pattern and track exist
        if song.pattern(pid).is_none() {
            return Err(McpBridgeError::PatternNotFound(pattern_id));
        }
        if song.track(tid).is_none() {
            return Err(McpBridgeError::TrackNotFound(track_id));
        }

        song.place_pattern(pid, tid, tick);
        Ok(())
    }

    fn remove_placement(
        &self,
        pattern_id: u32,
        track_id: u16,
        start_beat: f32,
    ) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let tid = synth_sequencer::TrackId(track_id);
        let tick = synth_sequencer::Tick(u64::from(beats_to_ticks(start_beat)));
        song.remove_placement(pid, tid, tick);
        Ok(())
    }

    fn list_arrangement(&self) -> Result<Vec<PlacementInfo>, McpBridgeError> {
        let song = self
            .shared
            .song
            .read()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        Ok(song
            .arrangement()
            .iter()
            .map(|p| PlacementInfo {
                pattern_id: p.pattern_id.0,
                track_id: p.track_id.0,
                start_beat: ticks_to_beats_u64(p.start.0),
            })
            .collect())
    }

    // === Sequencer: Batch operations ===

    fn add_notes(
        &self,
        pattern_id: u32,
        notes: &[BridgeNoteData],
    ) -> Result<BatchResult, McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let mut items = Vec::with_capacity(notes.len());
        let mut succeeded = 0usize;

        for (i, n) in notes.iter().enumerate() {
            let note_id = insert_note_into_pattern(pattern, n);
            items.push(BatchItemResult {
                index: i,
                success: true,
                id: Some(note_id),
                error: None,
            });
            succeeded += 1;
        }

        Ok(BatchResult {
            total: notes.len(),
            succeeded,
            failed: notes.len() - succeeded,
            items,
        })
    }

    fn update_notes(
        &self,
        pattern_id: u32,
        updates: &[BridgeNoteUpdate],
    ) -> Result<BatchResult, McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let mut items = Vec::with_capacity(updates.len());
        let mut succeeded = 0usize;

        for (i, u) in updates.iter().enumerate() {
            let nid = synth_sequencer::NoteId(u.note_id);
            if let Some(note) = pattern.note_mut(nid) {
                if let Some(p) = u.pitch
                    && let Some(new_pitch) = synth_sequencer::Pitch::new(p)
                {
                    note.pitch = new_pitch;
                }
                if let Some(s) = u.start_beat {
                    note.start = synth_sequencer::PatternTick(beats_to_ticks(s));
                }
                if let Some(d) = u.duration_beats {
                    note.duration = Some(synth_sequencer::Duration(beats_to_ticks(d)));
                }
                if let Some(v) = u.velocity {
                    note.velocity = synth_core::Velocity::from_midi(v);
                }
                items.push(BatchItemResult {
                    index: i,
                    success: true,
                    id: None,
                    error: None,
                });
                succeeded += 1;
            } else {
                items.push(BatchItemResult {
                    index: i,
                    success: false,
                    id: None,
                    error: Some(format!("note not found: {}", u.note_id)),
                });
            }
        }

        Ok(BatchResult {
            total: updates.len(),
            succeeded,
            failed: updates.len() - succeeded,
            items,
        })
    }

    fn replace_notes(
        &self,
        pattern_id: u32,
        notes: &[BridgeNoteData],
    ) -> Result<BatchResult, McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        pattern.clear_notes();

        let mut items = Vec::with_capacity(notes.len());
        let mut succeeded = 0usize;

        for (i, n) in notes.iter().enumerate() {
            let note_id = insert_note_into_pattern(pattern, n);
            items.push(BatchItemResult {
                index: i,
                success: true,
                id: Some(note_id),
                error: None,
            });
            succeeded += 1;
        }

        Ok(BatchResult {
            total: notes.len(),
            succeeded,
            failed: notes.len() - succeeded,
            items,
        })
    }

    fn clear_pattern(&self, pattern_id: u32) -> Result<usize, McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        let count = pattern.note_count();
        pattern.clear_notes();
        Ok(count)
    }

    fn create_patterns(
        &self,
        patterns: &[BridgePatternData],
    ) -> Result<BatchResult, McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;

        let mut items = Vec::with_capacity(patterns.len());
        let mut succeeded = 0usize;

        for (i, p) in patterns.iter().enumerate() {
            let duration = synth_sequencer::Duration(beats_to_ticks(p.length_beats));
            let id = song.create_pattern(duration);
            if let Some(pattern) = song.pattern_mut(id) {
                pattern.name = p.name.clone();
                for n in &p.notes {
                    insert_note_into_pattern(pattern, n);
                }
            }
            items.push(BatchItemResult {
                index: i,
                success: true,
                id: Some(u64::from(id.0)),
                error: None,
            });
            succeeded += 1;
        }

        Ok(BatchResult {
            total: patterns.len(),
            succeeded,
            failed: patterns.len() - succeeded,
            items,
        })
    }

    fn create_tracks(&self, tracks: &[BridgeTrackData]) -> Result<BatchResult, McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;

        let mut items = Vec::with_capacity(tracks.len());
        let mut succeeded = 0usize;

        for (i, t) in tracks.iter().enumerate() {
            let id = song.create_track(&t.name);
            if let Some(inst_id) = t.instrument_id
                && let Some(track) = song.track_mut(id)
            {
                track.instrument = Some(synth_sequencer::SeqInstrumentId(inst_id));
            }
            items.push(BatchItemResult {
                index: i,
                success: true,
                id: Some(u64::from(id.0)),
                error: None,
            });
            succeeded += 1;
        }

        Ok(BatchResult {
            total: tracks.len(),
            succeeded,
            failed: tracks.len() - succeeded,
            items,
        })
    }

    fn place_patterns(
        &self,
        placements: &[BridgePlacementData],
    ) -> Result<BatchResult, McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;

        let mut items = Vec::with_capacity(placements.len());
        let mut succeeded = 0usize;

        for (i, p) in placements.iter().enumerate() {
            let pid = synth_sequencer::PatternId::new(p.pattern_id);
            let tid = synth_sequencer::TrackId(p.track_id);
            let tick = synth_sequencer::Tick(u64::from(beats_to_ticks(p.start_beat)));

            if song.pattern(pid).is_none() {
                items.push(BatchItemResult {
                    index: i,
                    success: false,
                    id: None,
                    error: Some(format!("pattern not found: {}", p.pattern_id)),
                });
                continue;
            }
            if song.track(tid).is_none() {
                items.push(BatchItemResult {
                    index: i,
                    success: false,
                    id: None,
                    error: Some(format!("track not found: {}", p.track_id)),
                });
                continue;
            }

            song.place_pattern(pid, tid, tick);
            items.push(BatchItemResult {
                index: i,
                success: true,
                id: None,
                error: None,
            });
            succeeded += 1;
        }

        Ok(BatchResult {
            total: placements.len(),
            succeeded,
            failed: placements.len() - succeeded,
            items,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn set_song(
        &self,
        name: &str,
        tempo: f32,
        patterns: &[BridgePatternData],
        tracks: &[BridgeTrackData],
        placements: &[BridgeSongPlacement],
    ) -> Result<SetSongResult, McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;

        // Replace the entire song
        *song = synth_sequencer::Song::new(name).with_tempo(synth_core::Bpm::new(tempo));

        let mut errors = Vec::new();
        let mut total_notes = 0usize;

        // Create patterns with notes
        let mut pattern_ids: Vec<u32> = Vec::with_capacity(patterns.len());
        for (i, p) in patterns.iter().enumerate() {
            let duration = synth_sequencer::Duration(beats_to_ticks(p.length_beats));
            let id = song.create_pattern(duration);
            if let Some(pattern) = song.pattern_mut(id) {
                pattern.name = p.name.clone();
                for n in &p.notes {
                    insert_note_into_pattern(pattern, n);
                    total_notes += 1;
                }
            } else {
                errors.push(format!("failed to access pattern[{i}] after creation"));
            }
            pattern_ids.push(id.0);
        }

        // Create tracks
        let mut track_ids: Vec<u16> = Vec::with_capacity(tracks.len());
        for t in tracks {
            let id = song.create_track(&t.name);
            if let Some(inst_id) = t.instrument_id
                && let Some(track) = song.track_mut(id)
            {
                track.instrument = Some(synth_sequencer::SeqInstrumentId(inst_id));
            }
            track_ids.push(id.0);
        }

        // Create arrangement placements (index-based → real IDs)
        let mut placements_created = 0usize;
        for (i, pl) in placements.iter().enumerate() {
            if pl.pattern_index >= pattern_ids.len() {
                errors.push(format!(
                    "placement[{i}]: pattern_index {} out of range (have {})",
                    pl.pattern_index,
                    pattern_ids.len()
                ));
                continue;
            }
            if pl.track_index >= track_ids.len() {
                errors.push(format!(
                    "placement[{i}]: track_index {} out of range (have {})",
                    pl.track_index,
                    track_ids.len()
                ));
                continue;
            }

            let pid = synth_sequencer::PatternId::new(pattern_ids[pl.pattern_index]);
            let tid = synth_sequencer::TrackId(track_ids[pl.track_index]);
            let tick = synth_sequencer::Tick(u64::from(beats_to_ticks(pl.start_beat)));
            song.place_pattern(pid, tid, tick);
            placements_created += 1;
        }

        // Also update engine transport tempo
        drop(song);
        let _ = self
            .session
            .command_sender()
            .send(EngineCommand::SetTempo(synth_core::Bpm::new(tempo)));

        Ok(SetSongResult {
            patterns_created: pattern_ids.len(),
            tracks_created: track_ids.len(),
            notes_added: total_notes,
            placements_created,
            pattern_ids,
            track_ids,
            errors,
        })
    }

    // === Sequencer: Transport ===

    fn seq_play(&self) -> Result<(), McpBridgeError> {
        if self.session.command_sender().send(EngineCommand::Play) {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed)
        }
    }

    fn seq_stop(&self) -> Result<(), McpBridgeError> {
        if self.session.command_sender().send(EngineCommand::Stop) {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed)
        }
    }

    fn seq_seek(&self, beat: f32) -> Result<(), McpBridgeError> {
        let tick = synth_sequencer::Tick(u64::from(beats_to_ticks(beat)));
        if self
            .session
            .command_sender()
            .send(EngineCommand::Seek { tick })
        {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed)
        }
    }
}

/// Insert a note from `BridgeNoteData` into a pattern. Returns the assigned note ID as u64.
fn insert_note_into_pattern(pattern: &mut synth_sequencer::Pattern, n: &BridgeNoteData) -> u64 {
    let pitch = synth_sequencer::Pitch::new(n.pitch).unwrap_or(synth_sequencer::Pitch::MIDDLE_C);
    let start = synth_sequencer::PatternTick(beats_to_ticks(n.start_beat));
    let vel = synth_core::Velocity::from_midi(n.velocity);
    let instrument = synth_sequencer::SeqInstrumentId(0);

    let note = synth_sequencer::Note::new(
        synth_sequencer::NoteId(0), // reassigned by insert_note
        start,
        pitch,
        vel,
        instrument,
    )
    .with_duration(synth_sequencer::Duration(beats_to_ticks(n.duration_beats)));

    pattern.insert_note(note).0
}

/// Convert beats (float) to ticks (u32). 1 beat = 960 ticks.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn beats_to_ticks(beats: f32) -> u32 {
    (beats * synth_sequencer::TICKS_PER_QUARTER as f32).round() as u32
}

/// Convert ticks (u32) to beats (float).
#[allow(clippy::cast_precision_loss)]
fn ticks_to_beats(ticks: u32) -> f32 {
    ticks as f32 / synth_sequencer::TICKS_PER_QUARTER as f32
}

/// Convert ticks (u64) to beats (float).
#[allow(clippy::cast_precision_loss)]
fn ticks_to_beats_u64(ticks: u64) -> f32 {
    ticks as f32 / synth_sequencer::TICKS_PER_QUARTER as f32
}

/// Compute overlapping module pairs from their positions and sizes.
fn compute_overlaps(modules: &[crate::mcp_shared::ModuleLayout]) -> Vec<UiOverlap> {
    let mut overlaps = Vec::new();
    for i in 0..modules.len() {
        for j in (i + 1)..modules.len() {
            let a = &modules[i];
            let b = &modules[j];
            // Rectangle intersection
            let ax1 = a.position.0;
            let ay1 = a.position.1;
            let ax2 = ax1 + a.size.0;
            let ay2 = ay1 + a.size.1;
            let bx1 = b.position.0;
            let by1 = b.position.1;
            let bx2 = bx1 + b.size.0;
            let by2 = by1 + b.size.1;

            let overlap_x = (ax2.min(bx2) - ax1.max(bx1)).max(0.0);
            let overlap_y = (ay2.min(by2) - ay1.max(by1)).max(0.0);
            let area = overlap_x * overlap_y;

            if area > 0.0 {
                overlaps.push(UiOverlap {
                    module_a: a.id.clone(),
                    module_b: b.id.clone(),
                    overlap_area: area,
                });
            }
        }
    }
    overlaps
}

/// Format a parameter value for human-readable display.
fn format_param_display(param: &Param) -> String {
    let value = param.as_f32();
    let name_lower = param.name().to_ascii_lowercase();

    // Add units based on parameter type
    if name_lower.contains("frequency") || name_lower.contains("cutoff") || name_lower == "rate" {
        if value >= 1000.0 {
            format!("{:.1} kHz", value / 1000.0)
        } else {
            format!("{value:.1} Hz")
        }
    } else if name_lower.contains("time")
        || name_lower.contains("attack")
        || name_lower.contains("release")
        || name_lower.contains("decay")
    {
        if value >= 1.0 {
            format!("{value:.2} s")
        } else {
            format!("{:.1} ms", value * 1000.0)
        }
    } else if name_lower.contains("volume")
        || name_lower.contains("gain")
        || name_lower.contains("level")
        || name_lower.contains("master")
    {
        format!("{value:.2}")
    } else {
        format!("{value:.3}")
    }
}
