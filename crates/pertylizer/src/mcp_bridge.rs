//! Bridge between the MCP server and the running synth engine.
//!
//! `AppSynthBridge` implements `SynthBridge` by reading from `EngineState`
//! (shared graph, meters, transport) and sending commands via `CommandSender`.

use std::sync::Arc;

use synth_core::{MidiNote, NormalizedValue, Param, PortDirection, Velocity};
use synth_engine::EngineCommand;
use synth_engine::commands::ModuleId;
use synth_engine::instrument::{InstrumentId, MidiChannel};
use synth_mcp::bridge::SynthBridge;
use synth_mcp::bridge::{
    BridgeAutomationPointData, BridgeInstrumentDef, BridgeNoteData, BridgeNoteUpdate,
    BridgeParamSet, BridgeParamValue, BridgePatternData, BridgePlacementData, BridgeSongPlacement,
    BridgeTrackData,
};
use synth_mcp::error::McpBridgeError;
use synth_mcp::types::{
    ApplyExamplePatchResult, AutomationLaneInfo, AutomationPointInfo, BatchItemResult, BatchResult,
    BuildInstrumentResult, ConnectionInfo, DiagnosticSeverity, EngineStatus, ExamplePatchInfo,
    GraphDiagnostic, InstrumentInfo, ModuleInfo, ModuleTypeInfo, NoteInfo, OptimizeResult,
    ParamTypeInfo, ParameterInfo, PatchModuleInfo, PatchParamInfo, PatchParamValue,
    PatchResourceData, PatternInfo, PlacementInfo, SetSongResult, SongInfo, TrackInfo,
    UiConnectionInfo, UiModuleInfo, UiOverlap, UiSnapshot,
};

use crate::mcp_shared::McpSharedState;
use crate::session::SynthSession;

/// Bridge implementation for the Pertylizer application.
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
            let available: Vec<&str> = descriptor
                .ports
                .iter()
                .filter(|p| p.direction == direction)
                .map(|p| p.name.as_str())
                .collect();
            return Err(McpBridgeError::PortNotFound {
                module: module_str.to_string(),
                port: port.to_string(),
                available: format!("{available:?}"),
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
            category: snap.category.name().to_owned(),
            midi_channel: snap.midi_channel.as_u8(),
            volume: snap.volume.as_f32(),
            pan: snap.pan.as_f32(),
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
                let input_ports: Vec<String> = m
                    .input_connection_counts
                    .keys()
                    .map(|k| k.to_string())
                    .collect();
                let output_ports: Vec<String> = m
                    .output_connection_counts
                    .keys()
                    .map(|k| k.to_string())
                    .collect();

                // Build input/output ports from connections if snapshot doesn't have them
                let mut inputs = input_ports;
                let mut outputs = output_ports;

                for conn in &connections {
                    let to_port_str = conn.to_port.to_string();
                    if conn.to_module.to_string() == id_str && !inputs.contains(&to_port_str) {
                        inputs.push(to_port_str);
                    }
                    let from_port_str = conn.from_port.to_string();
                    if conn.from_module.to_string() == id_str && !outputs.contains(&from_port_str) {
                        outputs.push(from_port_str);
                    }
                }

                // Get descriptor for parameter ranges
                let descriptor = self.session.module_descriptor(inst_id, m.id);

                ModuleInfo {
                    id: id_str,
                    module_type: m.module_type.name().to_string(),
                    name: m.name.clone(),
                    bypassed: m.bypass_state == synth_core::BypassState::Bypassed,
                    parameters: m
                        .parameters
                        .iter()
                        .map(|p| {
                            let name = p.name().to_string();
                            let mut info = ParameterInfo {
                                name: name.clone(),
                                value: p.as_f32(),
                                display: format_param_display(p),
                                min: None,
                                max: None,
                                default: None,
                                choices: None,
                            };
                            // Enrich with range from descriptor
                            if let Some(ref desc) = descriptor {
                                let needle = normalize_param_name(&name);
                                if let Some(pd) = desc
                                    .parameters
                                    .iter()
                                    .find(|pd| normalize_param_name(&pd.name) == needle)
                                {
                                    info.min = Some(pd.range.min);
                                    info.max = Some(pd.range.max);
                                    info.default = Some(pd.range.default);
                                    info.choices = pd
                                        .choices
                                        .as_ref()
                                        .map(|c| c.iter().map(|ch| ch.name.clone()).collect());
                                }
                            }
                            info
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
                from_port: c.from_port.to_string(),
                to_module: c.to_module.to_string(),
                to_port: c.to_port.to_string(),
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
        let available: Vec<String> = module.parameters.iter().map(|p| p.name.clone()).collect();
        module
            .parameters
            .into_iter()
            .find(|p| p.name == param_name)
            .ok_or_else(|| {
                McpBridgeError::ParameterNotFound(format!(
                    "'{param_name}' not found, available: {available:?}"
                ))
            })
    }

    fn get_engine_status(&self) -> Result<EngineStatus, McpBridgeError> {
        let state = self.session.state();
        let (peak_left, peak_right) = state.meters.get_peak();
        let (rms_left, rms_right) = state.meters.get_rms();

        Ok(EngineStatus {
            cpu_usage: state.cpu_usage.load(),
            voice_count: state.voice_count.load(),
            sample_rate: state.sample_rate.load(),
            peak_left: peak_left.as_f32(),
            peak_right: peak_right.as_f32(),
            rms_left: rms_left.as_f32(),
            rms_right: rms_right.as_f32(),
            master_volume: state.master_volume.load(),
            tempo: state.transport.get_tempo().as_f32(),
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

        // Check for essential module types
        let has_sound_source = modules.iter().any(|m| {
            matches!(
                m.id.module_type,
                synth_core::ModuleType::Oscillator
                    | synth_core::ModuleType::MathOscillator
                    | synth_core::ModuleType::SubOscillator
                    | synth_core::ModuleType::Noise
                    | synth_core::ModuleType::WavetableOsc
                    | synth_core::ModuleType::AdditiveOsc
                    | synth_core::ModuleType::GranularOsc
                    | synth_core::ModuleType::FractalOsc
                    | synth_core::ModuleType::LaSynth
            )
        });
        let has_output = modules
            .iter()
            .any(|m| m.id.module_type == synth_core::ModuleType::StereoOutput);
        let has_envelope = modules
            .iter()
            .any(|m| m.id.module_type == synth_core::ModuleType::Envelope);

        if !has_sound_source {
            diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Error,
                module_id: None,
                message: "No sound source (oscillator/noise/granular) — instrument will be silent"
                    .to_string(),
            });
        }
        if !has_output {
            diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Error,
                module_id: None,
                message: "No stereo_output module — audio cannot reach the mixer".to_string(),
            });
        }
        if !has_envelope && has_sound_source {
            diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Warning,
                module_id: None,
                message: "No envelope — notes will have no attack/release shaping".to_string(),
            });
        }

        // Check for disconnected modules
        for module in &modules {
            let id_str = module.id.to_string();
            let has_input = connections
                .iter()
                .any(|c| c.to_module.to_string() == id_str);
            let has_output_conn = connections
                .iter()
                .any(|c| c.from_module.to_string() == id_str);

            // ModMatrix and KineticModulator work via parameters, not cables
            let works_via_params = matches!(
                module.id.module_type,
                synth_core::ModuleType::ModMatrix | synth_core::ModuleType::KineticModulator
            );

            if !has_input && !has_output_conn && !works_via_params {
                diagnostics.push(GraphDiagnostic {
                    severity: DiagnosticSeverity::Warning,
                    module_id: Some(id_str.clone()),
                    message: format!("Module {} ({}) has no connections", id_str, module.name),
                });
            } else if !has_output_conn
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
            category: "uncategorized".to_owned(),
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
            .set_instrument_volume(
                InstrumentId::new(instrument_id),
                synth_core::Gain::new(volume),
            )
            .map_err(|_| McpBridgeError::CommandSendFailed)
    }

    fn set_instrument_pan(&self, instrument_id: u64, pan: f32) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_pan(
                InstrumentId::new(instrument_id),
                synth_core::BipolarValue::new(pan),
            )
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
        let midi_channel = MidiChannel::from_one_indexed(channel).ok_or_else(|| {
            McpBridgeError::Other(format!("invalid MIDI channel {channel}, must be 1-16"))
        })?;
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

    fn set_instrument_category(
        &self,
        instrument_id: u64,
        category: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let cat: synth_engine::InstrumentCategory =
            category.parse().map_err(McpBridgeError::Other)?;
        self.session
            .set_instrument_category(InstrumentId::new(instrument_id), cat)
            .map_err(|_| McpBridgeError::CommandSendFailed)
    }

    fn set_parameter(
        &self,
        instrument_id: u64,
        module_id: &str,
        param_name: &str,
        value: f32,
    ) -> Result<ParameterInfo, McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let inst_id = InstrumentId::new(instrument_id);
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;

        // Use session.set_parameter for correct effect/module routing
        self.session
            .set_parameter(
                inst_id,
                mid,
                param_name,
                &crate::patch::ParamValue::Float(value),
            )
            .map_err(|e| match e {
                crate::session::SessionError::ModuleNotFound(s) => {
                    McpBridgeError::ModuleNotFound(s)
                }
                crate::session::SessionError::ParameterNotFound(s) => {
                    McpBridgeError::ParameterNotFound(s)
                }
                _ => McpBridgeError::CommandSendFailed,
            })?;

        // Read back the actual value directly from the descriptor (avoids listing all modules)
        let needle = normalize_param_name(param_name);
        let descriptor = self.session.module_descriptor(inst_id, mid);
        if let Some(desc) = descriptor
            && let Some(pd) = desc
                .parameters
                .iter()
                .find(|pd| normalize_param_name(&pd.name) == needle)
        {
            return Ok(ParameterInfo {
                name: pd.name.clone(),
                value: pd.id.as_f32(),
                display: format_param_display(&pd.id),
                min: Some(pd.range.min),
                max: Some(pd.range.max),
                default: Some(pd.range.default),
                choices: pd
                    .choices
                    .as_ref()
                    .map(|c| c.iter().map(|ch| ch.name.clone()).collect()),
            });
        }
        Ok(ParameterInfo {
            name: param_name.to_string(),
            value,
            display: format!("{value}"),
            min: None,
            max: None,
            default: None,
            choices: None,
        })
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

    fn get_example_patch(&self, name: &str) -> Result<PatchResourceData, McpBridgeError> {
        use crate::patch::ParamValue;

        let categories = crate::patches::categorized_patches();
        let name_lower = name.to_ascii_lowercase();

        for (category, patches) in &categories {
            for patch in patches {
                if patch.name.to_ascii_lowercase() == name_lower {
                    let modules = patch
                        .modules
                        .iter()
                        .map(|m| PatchModuleInfo {
                            id: m.id.clone(),
                            module_type: m.module_type.name().to_string(),
                            parameters: m
                                .parameters
                                .iter()
                                .map(|(k, v)| PatchParamInfo {
                                    name: k.clone(),
                                    value: match v {
                                        ParamValue::Float(f) => PatchParamValue::Float(*f),
                                        ParamValue::Int(i) => PatchParamValue::Int(*i),
                                        ParamValue::Bool(b) => PatchParamValue::Bool(*b),
                                        ParamValue::Choice(s) => PatchParamValue::Choice(s.clone()),
                                    },
                                })
                                .collect(),
                        })
                        .collect();

                    let connections = patch
                        .connections
                        .iter()
                        .map(|c| UiConnectionInfo {
                            from_module: c.from.0.clone(),
                            from_port: c.from.1.clone(),
                            to_module: c.to.0.clone(),
                            to_port: c.to.1.clone(),
                        })
                        .collect();

                    return Ok(PatchResourceData {
                        name: patch.name.clone(),
                        category: (*category).to_string(),
                        description: patch.description.clone().unwrap_or_default(),
                        tags: patch.tags.clone(),
                        modules,
                        connections,
                    });
                }
            }
        }

        Err(McpBridgeError::PatchNotFound(name.to_string()))
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

    fn request_auto_layout(&self) -> Result<String, McpBridgeError> {
        self.shared
            .pending_auto_layout
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok("OK: auto-layout queued for next frame".to_string())
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
                    .map(|p| p.name.to_string())
                    .collect();
                let output_ports = desc
                    .ports
                    .iter()
                    .filter(|p| p.direction == PortDirection::Output)
                    .map(|p| p.name.to_string())
                    .collect();
                let parameters = desc
                    .parameters
                    .iter()
                    .map(|p| ParamTypeInfo {
                        name: p.name.clone(),
                        choices: p
                            .choices
                            .as_ref()
                            .map(|opts| opts.iter().map(|c| c.id.clone()).collect()),
                    })
                    .collect();

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
        if length_beats <= 0.0 {
            return Err(McpBridgeError::Other(format!(
                "length_beats must be positive, got {length_beats}"
            )));
        }
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
        Ok(pattern.notes().iter().map(note_to_info).collect())
    }

    fn add_note(
        &self,
        pattern_id: u32,
        pitch: u8,
        start_beat: f32,
        duration_beats: f32,
        velocity: u8,
        instrument_id: Option<u16>,
    ) -> Result<NoteInfo, McpBridgeError> {
        if start_beat < 0.0 {
            return Err(McpBridgeError::Other(format!(
                "start_beat must be >= 0, got {start_beat}"
            )));
        }
        if duration_beats <= 0.0 {
            return Err(McpBridgeError::Other(format!(
                "duration_beats must be positive, got {duration_beats}"
            )));
        }
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let p = synth_sequencer::Pitch::new(pitch).ok_or_else(|| {
            McpBridgeError::Other(format!("invalid pitch {pitch}, must be 0-127"))
        })?;
        let start = synth_sequencer::PatternTick(beats_to_ticks(start_beat));
        let vel = synth_core::Velocity::from_midi(velocity);
        let instrument = synth_sequencer::SeqInstrumentId(instrument_id.unwrap_or(0));

        let note = synth_sequencer::Note::new(
            synth_sequencer::NoteId(0), // will be reassigned by insert_note
            start,
            p,
            vel,
            instrument,
        )
        .with_duration(synth_sequencer::Duration(beats_to_ticks(duration_beats)));

        let note_id = pattern.insert_note(note);
        // Read back the inserted note to return full info
        Ok(pattern.note(note_id).map(note_to_info).unwrap_or(NoteInfo {
            id: note_id.0,
            pitch,
            pitch_name: p.to_string(),
            start_beat,
            duration_beats,
            velocity,
        }))
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
    ) -> Result<NoteInfo, McpBridgeError> {
        if let Some(s) = start_beat
            && s < 0.0
        {
            return Err(McpBridgeError::Other(format!(
                "start_beat must be >= 0, got {s}"
            )));
        }
        if let Some(d) = duration_beats
            && d <= 0.0
        {
            return Err(McpBridgeError::Other(format!(
                "duration_beats must be positive, got {d}"
            )));
        }
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

        if let Some(p) = pitch {
            if let Some(new_pitch) = synth_sequencer::Pitch::new(p) {
                note.pitch = new_pitch;
            } else {
                return Err(McpBridgeError::Other(format!(
                    "invalid pitch {p}, must be 0-127"
                )));
            }
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

        Ok(note_to_info(note))
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
                insert_automation_into_pattern(pattern, &p.automation);
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
                insert_automation_into_pattern(pattern, &p.automation);
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

    fn build_instrument(
        &self,
        spec: &BridgeInstrumentDef,
    ) -> Result<BuildInstrumentResult, McpBridgeError> {
        use crate::patch::ParamValue;

        // 1. Create or reuse instrument
        let inst_id = if let Some(id) = spec.instrument_id {
            let iid = InstrumentId::new(id);
            if !self.session.instrument_exists(iid) {
                return Err(McpBridgeError::InstrumentNotFound(id));
            }
            // Clear existing graph before rebuilding
            self.session
                .clear_graph(iid)
                .map_err(|e| McpBridgeError::Other(e.to_string()))?;
            // Rename
            let _ = self.session.rename_instrument(iid, &spec.name);
            iid
        } else {
            self.session
                .add_instrument(&spec.name)
                .map_err(|e| McpBridgeError::Other(e.to_string()))?
        };

        let mut errors = Vec::new();

        // 2. Set optional instrument params
        if let Some(ch) = spec.midi_channel {
            let _ = self
                .session
                .set_instrument_midi_channel(
                    inst_id,
                    MidiChannel::from_one_indexed(ch).unwrap_or(MidiChannel::CH1),
                )
                .map_err(|e| errors.push(format!("midi_channel: {e}")));
        }
        if let Some(vol) = spec.volume {
            let _ = self
                .session
                .set_instrument_volume(inst_id, synth_core::Gain::new(vol))
                .map_err(|e| errors.push(format!("volume: {e}")));
        }
        if let Some(pan) = spec.pan {
            let _ = self
                .session
                .set_instrument_pan(inst_id, synth_core::BipolarValue::new(pan))
                .map_err(|e| errors.push(format!("pan: {e}")));
        }

        // 3. Add modules (keep descriptors for port validation)
        let mut module_ids: Vec<Option<ModuleId>> = Vec::with_capacity(spec.modules.len());
        let mut module_id_strings: Vec<String> = Vec::with_capacity(spec.modules.len());
        let mut module_descriptors: Vec<Option<synth_core::ModuleDescriptor>> =
            Vec::with_capacity(spec.modules.len());

        for module_def in &spec.modules {
            let mt = match synth_core::ModuleType::from_prefix(&module_def.module_type) {
                Some(mt) => mt,
                None => {
                    errors.push(format!("invalid module type: {}", module_def.module_type));
                    module_ids.push(None);
                    module_id_strings.push(String::new());
                    module_descriptors.push(None);
                    continue;
                }
            };

            match self.session.add_module(inst_id, mt) {
                Ok((mid, descriptor)) => {
                    let mid_str = mid.to_string();

                    // Set parameters
                    for (param_name, value) in &module_def.params {
                        let pv = match value {
                            BridgeParamValue::Number(n) => ParamValue::Float(*n as f32),
                            BridgeParamValue::Choice(s) => ParamValue::Choice(s.clone()),
                            BridgeParamValue::Bool(b) => ParamValue::Bool(*b),
                        };
                        if let Err(e) = self.session.set_parameter(inst_id, mid, param_name, &pv) {
                            errors.push(format!("{mid} param '{param_name}': {e}"));
                        }
                    }

                    module_ids.push(Some(mid));
                    module_id_strings.push(mid_str);
                    module_descriptors.push(Some(descriptor));
                }
                Err(e) => {
                    errors.push(format!("add module '{}': {e}", module_def.module_type));
                    module_ids.push(None);
                    module_id_strings.push(String::new());
                    module_descriptors.push(None);
                }
            }
        }

        // 4. Wire connections (with port validation)
        let mut connection_count = 0;
        for conn in &spec.connections {
            let from_mid = match module_ids.get(conn.from_index).and_then(|m| *m) {
                Some(id) => id,
                None => {
                    errors.push(format!(
                        "connection from_index {} has no module",
                        conn.from_index
                    ));
                    continue;
                }
            };
            let to_mid = match module_ids.get(conn.to_index).and_then(|m| *m) {
                Some(id) => id,
                None => {
                    errors.push(format!(
                        "connection to_index {} has no module",
                        conn.to_index
                    ));
                    continue;
                }
            };

            // Validate source port exists and is an output
            if let Some(Some(desc)) = module_descriptors.get(conn.from_index) {
                let has_output = desc.ports.iter().any(|p| {
                    p.name.as_str() == conn.from_port
                        && p.direction == synth_core::PortDirection::Output
                });
                if !has_output {
                    let available: Vec<&str> = desc
                        .ports
                        .iter()
                        .filter(|p| p.direction == synth_core::PortDirection::Output)
                        .map(|p| p.name.as_str())
                        .collect();
                    errors.push(format!(
                        "{from_mid} has no output port '{}', available: {available:?}",
                        conn.from_port
                    ));
                    continue;
                }
            }

            // Validate destination port exists and is an input
            if let Some(Some(desc)) = module_descriptors.get(conn.to_index) {
                let has_input = desc.ports.iter().any(|p| {
                    p.name.as_str() == conn.to_port
                        && p.direction == synth_core::PortDirection::Input
                });
                if !has_input {
                    let available: Vec<&str> = desc
                        .ports
                        .iter()
                        .filter(|p| p.direction == synth_core::PortDirection::Input)
                        .map(|p| p.name.as_str())
                        .collect();
                    errors.push(format!(
                        "{to_mid} has no input port '{}', available: {available:?}",
                        conn.to_port
                    ));
                    continue;
                }
            }

            match self.session.connect(
                inst_id,
                from_mid,
                conn.from_port.clone(),
                to_mid,
                conn.to_port.clone(),
            ) {
                Ok(()) => connection_count += 1,
                Err(e) => errors.push(format!(
                    "connect {}:{} → {}:{}: {e}",
                    from_mid, conn.from_port, to_mid, conn.to_port
                )),
            }
        }

        Ok(BuildInstrumentResult {
            instrument_id: inst_id.as_u64(),
            module_ids: module_id_strings,
            connection_count,
            errors,
            hint: Some(format!(
                "Run get_graph_diagnostics(instrument_id: {}) to validate the instrument",
                inst_id.as_u64()
            )),
        })
    }

    fn build_instruments(
        &self,
        specs: &[BridgeInstrumentDef],
    ) -> Result<Vec<BuildInstrumentResult>, McpBridgeError> {
        let mut results = Vec::with_capacity(specs.len());
        for spec in specs {
            results.push(self.build_instrument(spec)?);
        }
        Ok(results)
    }

    fn apply_example_patch(
        &self,
        instrument_id: Option<u64>,
        patch_name: &str,
    ) -> Result<ApplyExamplePatchResult, McpBridgeError> {
        // Find the patch
        let categories = crate::patches::categorized_patches();
        let name_lower = patch_name.to_ascii_lowercase();

        let patch = categories
            .iter()
            .flat_map(|(_cat, patches)| patches)
            .find(|p| p.name.to_ascii_lowercase() == name_lower)
            .ok_or_else(|| McpBridgeError::PatchNotFound(patch_name.to_string()))?
            .clone();

        // Create or reuse instrument
        let inst_id = if let Some(id) = instrument_id {
            let iid = InstrumentId::new(id);
            if !self.session.instrument_exists(iid) {
                return Err(McpBridgeError::InstrumentNotFound(id));
            }
            iid
        } else {
            self.session
                .add_instrument(&patch.name)
                .map_err(|e| McpBridgeError::Other(e.to_string()))?
        };

        let result = self.session.apply_patch(inst_id, &patch);

        Ok(ApplyExamplePatchResult {
            instrument_id: inst_id.as_u64(),
            patch_name: patch.name,
            module_count: result.module_count,
            connection_count: result.connection_count,
            errors: result.errors,
        })
    }

    fn add_automation_points(
        &self,
        pattern_id: u32,
        points: &[BridgeAutomationPointData],
    ) -> Result<BatchResult, McpBridgeError> {
        use synth_sequencer::{AutomationPoint, AutomationTarget, PatternTick, SeqInstrumentId};

        let mut song_w = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pat_id = synth_sequencer::PatternId(pattern_id);
        let pattern = song_w
            .pattern_mut(pat_id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let mut succeeded = 0usize;
        let mut items = Vec::new();
        let total = points.len();

        for (i, pt) in points.iter().enumerate() {
            let param = match parse_auto_instrument_param(&pt.param) {
                Some(p) => p,
                None => {
                    items.push(BatchItemResult {
                        index: i,
                        success: false,
                        id: None,
                        error: Some(format!("unknown param '{}'", pt.param)),
                    });
                    continue;
                }
            };

            let target = AutomationTarget::Instrument {
                instrument: SeqInstrumentId::new(pt.instrument_id),
                param,
            };
            let tick = PatternTick(beats_to_ticks(pt.beat));
            let curve = parse_curve_type(&pt.curve);
            let lane = pattern.get_or_create_automation(target);
            lane.add_point(
                AutomationPoint::new(tick, NormalizedValue::new(pt.value)).with_curve(curve),
            );
            items.push(BatchItemResult {
                index: i,
                success: true,
                id: None,
                error: None,
            });
            succeeded += 1;
        }

        Ok(BatchResult {
            total,
            succeeded,
            failed: total - succeeded,
            items,
        })
    }

    fn list_automation_lanes(
        &self,
        pattern_id: u32,
    ) -> Result<Vec<AutomationLaneInfo>, McpBridgeError> {
        let song = self
            .shared
            .song
            .read()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pat_id = synth_sequencer::PatternId(pattern_id);
        let pattern = song
            .pattern(pat_id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        Ok(pattern
            .automation
            .iter()
            .map(|lane| {
                let (target_name, instrument_id) = automation_target_info(&lane.target);
                AutomationLaneInfo {
                    target: target_name,
                    instrument_id,
                    point_count: lane.len(),
                }
            })
            .collect())
    }

    fn get_automation_points(
        &self,
        pattern_id: u32,
        target: &str,
        instrument_id: u16,
    ) -> Result<Vec<AutomationPointInfo>, McpBridgeError> {
        let song = self
            .shared
            .song
            .read()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pat_id = synth_sequencer::PatternId(pattern_id);
        let pattern = song
            .pattern(pat_id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let auto_target = build_automation_target(target, instrument_id)?;
        let lane = pattern
            .automation_lane(&auto_target)
            .ok_or_else(|| McpBridgeError::Other(format!("automation lane not found: {target}")))?;

        Ok(lane
            .points()
            .iter()
            .map(|p| AutomationPointInfo {
                beat: ticks_to_beats(p.tick.0),
                value: p.value.as_f32(),
                curve: format_curve_type(p.curve),
            })
            .collect())
    }

    fn remove_automation_points(
        &self,
        pattern_id: u32,
        target: &str,
        instrument_id: u16,
        beats: &[f32],
    ) -> Result<BatchResult, McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pat_id = synth_sequencer::PatternId(pattern_id);
        let pattern = song
            .pattern_mut(pat_id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let auto_target = build_automation_target(target, instrument_id)?;
        let lane = pattern.get_or_create_automation(auto_target);

        let total = beats.len();
        let mut succeeded = 0usize;
        let mut items = Vec::with_capacity(total);

        for (i, &beat) in beats.iter().enumerate() {
            let tick = synth_sequencer::PatternTick(beats_to_ticks(beat));
            if lane.remove_point(tick).is_some() {
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
                    error: Some(format!("no point at beat {beat}")),
                });
            }
        }

        Ok(BatchResult {
            total,
            succeeded,
            failed: total - succeeded,
            items,
        })
    }

    fn clear_automation_lane(
        &self,
        pattern_id: u32,
        target: &str,
        instrument_id: u16,
    ) -> Result<usize, McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pat_id = synth_sequencer::PatternId(pattern_id);
        let pattern = song
            .pattern_mut(pat_id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let auto_target = build_automation_target(target, instrument_id)?;
        let lane = pattern.get_or_create_automation(auto_target);
        let count = lane.len();
        lane.clear();
        Ok(count)
    }

    // === Track control ===

    fn set_track_volume(&self, track_id: u16, volume: f32) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let tid = synth_sequencer::TrackId(track_id);
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.volume = synth_core::NormalizedValue::new(volume);
        Ok(())
    }

    fn set_track_pan(&self, track_id: u16, pan: f32) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let tid = synth_sequencer::TrackId(track_id);
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.pan = synth_core::NormalizedValue::new(pan);
        Ok(())
    }

    fn set_track_mute(&self, track_id: u16, muted: bool) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let tid = synth_sequencer::TrackId(track_id);
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.mute = muted;
        Ok(())
    }

    fn set_track_solo(&self, track_id: u16, solo: bool) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let tid = synth_sequencer::TrackId(track_id);
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.solo = solo;
        Ok(())
    }

    fn set_track_instrument(
        &self,
        track_id: u16,
        instrument_id: Option<u16>,
    ) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let tid = synth_sequencer::TrackId(track_id);
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.instrument = instrument_id.map(synth_sequencer::SeqInstrumentId);
        Ok(())
    }

    fn rename_track(&self, track_id: u16, name: &str) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let tid = synth_sequencer::TrackId(track_id);
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.name = name.to_string();
        Ok(())
    }

    fn delete_track(&self, track_id: u16) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let tid = synth_sequencer::TrackId(track_id);
        song.delete_track(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        Ok(())
    }

    // === Pattern management ===

    fn rename_pattern(&self, pattern_id: u32, name: &str) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        pattern.name = name.to_string();
        Ok(())
    }

    fn set_pattern_length(&self, pattern_id: u32, length_beats: f32) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        pattern.length = synth_sequencer::Duration(beats_to_ticks(length_beats));
        Ok(())
    }

    fn duplicate_pattern(&self, pattern_id: u32) -> Result<u32, McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        let pid = synth_sequencer::PatternId::new(pattern_id);
        song.duplicate_pattern(pid)
            .map(|new_id| new_id.0)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))
    }

    // === Song metadata ===

    fn set_song_author(&self, author: &str) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        song.author = author.to_string();
        Ok(())
    }

    fn set_song_time_signature(
        &self,
        numerator: u8,
        denominator: u8,
    ) -> Result<(), McpBridgeError> {
        let mut song = self
            .shared
            .song
            .write()
            .map_err(|_| McpBridgeError::SongLockPoisoned)?;
        song.default_time_signature = synth_sequencer::TimeSignature {
            numerator,
            denominator,
        };
        Ok(())
    }

    // === Batch parameter set ===

    fn set_parameters(
        &self,
        instrument_id: u64,
        params: &[BridgeParamSet],
    ) -> Result<BatchResult, McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let inst_id = InstrumentId::new(instrument_id);

        let total = params.len();
        let mut succeeded = 0usize;
        let mut items = Vec::with_capacity(total);

        for (i, ps) in params.iter().enumerate() {
            let mid: ModuleId = match ps.module_id.parse() {
                Ok(id) => id,
                Err(_) => {
                    items.push(BatchItemResult {
                        index: i,
                        success: false,
                        id: None,
                        error: Some(format!("invalid module ID: {}", ps.module_id)),
                    });
                    continue;
                }
            };

            match self.session.set_parameter(
                inst_id,
                mid,
                &ps.param_name,
                &crate::patch::ParamValue::Float(ps.value),
            ) {
                Ok(()) => {
                    items.push(BatchItemResult {
                        index: i,
                        success: true,
                        id: None,
                        error: None,
                    });
                    succeeded += 1;
                }
                Err(e) => {
                    items.push(BatchItemResult {
                        index: i,
                        success: false,
                        id: None,
                        error: Some(format!("{}", e)),
                    });
                }
            }
        }

        Ok(BatchResult {
            total,
            succeeded,
            failed: total - succeeded,
            items,
        })
    }

    // === Project management ===

    fn new_project(&self) -> Result<String, McpBridgeError> {
        self.submit_project_action(crate::mcp_shared::ProjectAction::New)
    }

    fn save_project(&self, path: &str) -> Result<String, McpBridgeError> {
        let path = std::path::PathBuf::from(path);
        self.submit_project_action(crate::mcp_shared::ProjectAction::Save(path))
    }

    fn load_project(&self, path: &str) -> Result<String, McpBridgeError> {
        let path = std::path::PathBuf::from(path);
        if !path.exists() {
            return Err(McpBridgeError::Other(format!(
                "File not found: {}",
                path.display()
            )));
        }
        self.submit_project_action(crate::mcp_shared::ProjectAction::Load(path))
    }

    fn optimize_project(&self) -> Result<OptimizeResult, McpBridgeError> {
        // Remove unused patterns and tracks from the song
        let (removed_patterns, removed_tracks, used_instrument_ids) = {
            let mut song = self
                .shared
                .song
                .write()
                .map_err(|_| McpBridgeError::SongLockPoisoned)?;
            song.remove_unused()
        };

        // Remove instruments not referenced by remaining tracks/notes
        let mut removed_instruments = Vec::new();
        let snapshots = self.session.list_instruments();
        for snap in &snapshots {
            #[allow(clippy::cast_possible_truncation)]
            if !used_instrument_ids.contains(&(snap.id.as_u64() as u16)) {
                removed_instruments.push(snap.name.clone());
                let _ = self.session.remove_instrument(snap.id);
            }
        }

        let total_removed =
            removed_patterns.len() + removed_tracks.len() + removed_instruments.len();

        Ok(OptimizeResult {
            removed_patterns,
            removed_tracks,
            removed_instruments,
            total_removed,
        })
    }

    fn render_note_preview(
        &self,
        instrument_id: u64,
        note: u8,
        velocity: u8,
        duration_ms: u32,
        tail_ms: u32,
    ) -> Result<synth_mcp::types::AudioPreview, McpBridgeError> {
        crate::audio::preview::render_note_preview(
            &self.session,
            InstrumentId::new(instrument_id),
            MidiNote::new(note),
            Velocity::from_midi(velocity),
            duration_ms,
            tail_ms,
        )
    }
}

impl AppSynthBridge {
    /// Submit a project action to the GUI thread and wait for the result.
    fn submit_project_action(
        &self,
        action: crate::mcp_shared::ProjectAction,
    ) -> Result<String, McpBridgeError> {
        // Clear any stale result
        {
            let (lock, _) = &self.shared.project_action_result;
            if let Ok(mut guard) = lock.lock() {
                *guard = None;
            }
        }

        // Queue the action
        if let Ok(mut pending) = self.shared.pending_project_action.lock() {
            *pending = Some(action);
        } else {
            return Err(McpBridgeError::Other(
                "Failed to queue project action".to_string(),
            ));
        }

        // Wait for the GUI to process and signal the result (timeout 5s)
        let (lock, cvar) = &self.shared.project_action_result;
        let guard = lock
            .lock()
            .map_err(|e| McpBridgeError::Other(format!("Lock error: {e}")))?;
        let timeout = std::time::Duration::from_secs(5);
        let (mut guard, wait_result) = cvar
            .wait_timeout_while(
                guard,
                timeout,
                |result: &mut Option<Result<String, String>>| result.is_none(),
            )
            .map_err(|e| McpBridgeError::Other(format!("Wait error: {e}")))?;

        if wait_result.timed_out() {
            return Err(McpBridgeError::Other(
                "Timeout waiting for GUI to process project action".to_string(),
            ));
        }

        match guard.take() {
            Some(Ok(msg)) => Ok(msg),
            Some(Err(e)) => Err(McpBridgeError::Other(e)),
            None => Err(McpBridgeError::Other(
                "No result from project action".to_string(),
            )),
        }
    }
}

/// Insert a note from `BridgeNoteData` into a pattern. Returns the assigned note ID as u64.
fn insert_note_into_pattern(pattern: &mut synth_sequencer::Pattern, n: &BridgeNoteData) -> u64 {
    let pitch = synth_sequencer::Pitch::new(n.pitch).unwrap_or(synth_sequencer::Pitch::MIDDLE_C);
    let start = synth_sequencer::PatternTick(beats_to_ticks(n.start_beat));
    let vel = synth_core::Velocity::from_midi(n.velocity);
    let instrument = synth_sequencer::SeqInstrumentId(n.instrument_id.unwrap_or(0));

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

/// Normalize a parameter name for fuzzy matching (lowercase, underscores → spaces).
fn normalize_param_name(s: &str) -> String {
    s.to_lowercase().replace('_', " ")
}

/// Convert a sequencer `Note` to MCP `NoteInfo`.
fn note_to_info(n: &synth_sequencer::Note) -> NoteInfo {
    NoteInfo {
        id: n.id.0,
        pitch: n.pitch.as_midi(),
        pitch_name: n.pitch.to_string(),
        start_beat: ticks_to_beats(n.start.0),
        duration_beats: n.duration.map_or(1.0, |d| ticks_to_beats(d.0)),
        velocity: synth_core::Velocity::to_midi(n.velocity),
    }
}

/// Convert ticks (u64) to beats (float).
#[allow(clippy::cast_precision_loss)]
fn ticks_to_beats_u64(ticks: u64) -> f32 {
    ticks as f32 / synth_sequencer::TICKS_PER_QUARTER as f32
}

/// Parse a parameter name string to `AutoInstrumentParam`.
fn parse_auto_instrument_param(name: &str) -> Option<synth_sequencer::AutoInstrumentParam> {
    use synth_sequencer::AutoInstrumentParam;
    match name {
        "Volume" => Some(AutoInstrumentParam::Volume),
        "Pan" => Some(AutoInstrumentParam::Pan),
        "FilterCutoff" => Some(AutoInstrumentParam::FilterCutoff),
        "FilterResonance" => Some(AutoInstrumentParam::FilterResonance),
        "Attack" => Some(AutoInstrumentParam::Attack),
        "Decay" => Some(AutoInstrumentParam::Decay),
        "Sustain" => Some(AutoInstrumentParam::Sustain),
        "Release" => Some(AutoInstrumentParam::Release),
        _ => None,
    }
}

/// Parse a curve type string.
fn parse_curve_type(s: &str) -> synth_sequencer::CurveType {
    use synth_sequencer::CurveType;
    match s {
        "Step" => CurveType::Step,
        "Exponential" => CurveType::Exponential(0),
        "SCurve" => CurveType::SCurve,
        _ => CurveType::Linear,
    }
}

/// Format a `CurveType` to a string.
fn format_curve_type(curve: synth_sequencer::CurveType) -> String {
    use synth_sequencer::CurveType;
    match curve {
        CurveType::Linear => "Linear".to_string(),
        CurveType::Step => "Step".to_string(),
        CurveType::Exponential(strength) => format!("Exponential({strength})"),
        CurveType::SCurve => "SCurve".to_string(),
    }
}

/// Build an `AutomationTarget` from parameter name and instrument ID.
fn build_automation_target(
    target: &str,
    instrument_id: u16,
) -> Result<synth_sequencer::AutomationTarget, McpBridgeError> {
    let param = parse_auto_instrument_param(target)
        .ok_or_else(|| McpBridgeError::Other(format!("unknown automation param: {target}")))?;
    Ok(synth_sequencer::AutomationTarget::Instrument {
        instrument: synth_sequencer::SeqInstrumentId::new(instrument_id),
        param,
    })
}

/// Extract target name and optional instrument ID from an `AutomationTarget`.
fn automation_target_info(target: &synth_sequencer::AutomationTarget) -> (String, Option<u16>) {
    use synth_sequencer::AutoInstrumentParam;
    match target {
        synth_sequencer::AutomationTarget::Instrument { instrument, param } => {
            let name = match param {
                AutoInstrumentParam::Volume => "Volume",
                AutoInstrumentParam::Pan => "Pan",
                AutoInstrumentParam::FilterCutoff => "FilterCutoff",
                AutoInstrumentParam::FilterResonance => "FilterResonance",
                AutoInstrumentParam::Attack => "Attack",
                AutoInstrumentParam::Decay => "Decay",
                AutoInstrumentParam::Sustain => "Sustain",
                AutoInstrumentParam::Release => "Release",
            };
            (name.to_string(), Some(instrument.0))
        }
        synth_sequencer::AutomationTarget::Track { track, param } => {
            (format!("{param:?}"), Some(track.0))
        }
        synth_sequencer::AutomationTarget::Global(param) => (format!("{param:?}"), None),
    }
}

/// Insert automation points from `BridgeAutomationPointData` into a pattern.
fn insert_automation_into_pattern(
    pattern: &mut synth_sequencer::Pattern,
    points: &[BridgeAutomationPointData],
) {
    use synth_sequencer::{AutomationPoint, AutomationTarget, PatternTick, SeqInstrumentId};

    for pt in points {
        let Some(param) = parse_auto_instrument_param(&pt.param) else {
            continue;
        };
        let target = AutomationTarget::Instrument {
            instrument: SeqInstrumentId::new(pt.instrument_id),
            param,
        };
        let tick = PatternTick(beats_to_ticks(pt.beat));
        let curve = parse_curve_type(&pt.curve);
        let lane = pattern.get_or_create_automation(target);
        lane.add_point(
            AutomationPoint::new(tick, NormalizedValue::new(pt.value)).with_curve(curve),
        );
    }
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
