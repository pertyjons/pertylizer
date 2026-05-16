//! Bridge between the MCP server and the running synth engine.
//!
//! `AppSynthBridge` implements `SynthBridge` by reading from `EngineState`
//! (shared graph, meters, transport) and sending commands via `CommandSender`.

use std::collections::HashSet;
use std::sync::Arc;

use synth_core::{
    BipolarValue, Gain, MidiNote, ModMatrixParam, ModSource, NormalizedValue, Param, ParameterUnit,
    PortDirection, SampleCount, Velocity,
};
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
    AnalyzeHarmonyResult, AnalyzeMaskingMatrixResult, AnalyzeMixBusResult, AnalyzePatternResult,
    AnalyzeSectionResult, ApplyExamplePatchResult, AutomationLaneInfo, AutomationPointInfo,
    AweLfoInfo, AwePresetInfo, AweStateInfo, BandOverlap, BatchItemResult, BatchResult,
    BuildInstrumentResult, ConnectionCheckResult, ConnectionInfo, DiagnosticSeverity, EngineStatus,
    ExamplePatchInfo, GraphDiagnostic, HarmonyChordEvent, HarmonyKeyEstimate, HarmonyScope,
    HarmonyStats, InstrumentInfo, MaskingPair, MixBusMetrics, ModuleInfo, ModuleTypeInfo, NoteInfo,
    OptimizeResult, ParamTypeInfo, ParameterInfo, PatchModuleInfo, PatchParamInfo, PatchParamValue,
    PatchResourceData, PatternInfo, PlacementInfo, SetSongResult, SongInfo, TrackInfo,
    UiConnectionInfo, UiModuleInfo, UiOverlap, UiSnapshot,
};

use crate::mcp_shared::McpSharedState;
use crate::session::SynthSession;

/// Bridge implementation for the Pertylizer application.
pub struct AppSynthBridge {
    session: Arc<SynthSession>,
    shared: Arc<McpSharedState>,
    sample_library: Arc<std::sync::RwLock<synth_sampler::SampleLibrary>>,
}

impl AppSynthBridge {
    /// Create a new bridge with access to the session, shared MCP state, and sample library.
    pub fn new(
        session: Arc<SynthSession>,
        shared: Arc<McpSharedState>,
        sample_library: Arc<std::sync::RwLock<synth_sampler::SampleLibrary>>,
    ) -> Self {
        Self {
            session,
            shared,
            sample_library,
        }
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
            description: snap.description.clone(),
            patch_description: snap.patch_description.clone(),
            sidechain_source_id: snap.sidechain_source_id.map(|id| id.as_u64()),
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

    fn get_instrument_profiles(
        &self,
    ) -> Result<Vec<synth_mcp::types::InstrumentProfileResult>, McpBridgeError> {
        let song = self.shared.song.read();
        let profiles = crate::analysis::infer_all_profiles(&song, self.session.state());
        Ok(profiles.into_iter().map(profile_to_result).collect())
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

                // Get descriptor for parameter ranges and units
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
                            // Match against the descriptor first so we can format the value
                            // with its declared unit (the name-based fallback misformats
                            // e.g. EFL attack/release stored as Milliseconds).
                            let name = p.name().to_string();
                            let pd = descriptor.as_ref().and_then(|desc| {
                                let needle = normalize_param_name(&name);
                                desc.parameters
                                    .iter()
                                    .find(|pd| normalize_param_name(&pd.name) == needle)
                            });
                            ParameterInfo {
                                name: name.clone(),
                                value: p.as_f32(),
                                display: format_param_display(p, pd.map(|pd| pd.unit)),
                                min: pd.map(|pd| pd.range.min),
                                max: pd.map(|pd| pd.range.max),
                                default: pd.map(|pd| pd.range.default),
                                choices: pd.and_then(|pd| {
                                    pd.choices
                                        .as_ref()
                                        .map(|c| c.iter().map(|ch| ch.name.clone()).collect())
                                }),
                            }
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
                    | synth_core::ModuleType::Sampler
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

        // Collect module IDs referenced as sources by any Mod Matrix slot.
        // The Mod Matrix routes via parameter slots (not cables), so these
        // modules are "in use" even with no cable connections.
        let mod_matrix_sources = collect_mod_matrix_sources(&modules);

        // Check for disconnected modules
        for module in &modules {
            let id_str = module.id.to_string();

            // Effect-chain modules are auto-wired in series outside the voice
            // graph — they legitimately have no voice-graph cables.
            if module.id.module_type.is_effect() {
                continue;
            }

            // ModMatrix and KineticModulator route via parameters, not cables.
            if matches!(
                module.id.module_type,
                synth_core::ModuleType::ModMatrix | synth_core::ModuleType::KineticModulator
            ) {
                continue;
            }

            // Modulator referenced by a Mod Matrix slot is in use.
            if mod_matrix_sources.contains(&id_str) {
                continue;
            }

            let has_input = module.has_inputs();
            let has_output_conn = module.has_outputs();

            if !has_input && !has_output_conn {
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
        let id =
            self.session
                .add_instrument(name)
                .map_err(|_| McpBridgeError::CommandSendFailed {
                    command: "create_instrument",
                })?;

        // Return basic info — the engine will update the snapshots asynchronously
        Ok(InstrumentInfo {
            id: id.as_u64(),
            name: name.to_string(),
            description: String::new(),
            patch_description: None,
            sidechain_source_id: None,
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
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "delete_instrument",
            })
    }

    fn rename_instrument(&self, instrument_id: u64, name: &str) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .rename_instrument(InstrumentId::new(instrument_id), name)
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn set_instrument_description(
        &self,
        instrument_id: u64,
        description: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_description(InstrumentId::new(instrument_id), description)
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn set_patch_description(
        &self,
        instrument_id: u64,
        description: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let value = if description.is_empty() {
            None
        } else {
            Some(description)
        };
        self.session
            .set_patch_description(InstrumentId::new(instrument_id), value)
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn set_sidechain_source(
        &self,
        instrument_id: u64,
        source: Option<u64>,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        if let Some(src) = source {
            if src == instrument_id {
                return Err(McpBridgeError::Other(
                    "sidechain source must differ from the target instrument".into(),
                ));
            }
            self.validate_instrument(src)?;
            // Walk the current chain from `src` to detect cycles.
            // The engine also rejects cycles defensively, but pre-checking
            // here lets MCP report a clear error to the caller.
            let snapshots = self.session.list_instruments();
            let chain_len = snapshots.len();
            let mut current = Some(src);
            for _ in 0..=chain_len {
                let Some(id) = current else {
                    break;
                };
                if id == instrument_id {
                    return Err(McpBridgeError::Other(format!(
                        "sidechain would form a cycle through instrument {id}"
                    )));
                }
                current = snapshots
                    .iter()
                    .find(|s| s.id.as_u64() == id)
                    .and_then(|s| s.sidechain_source_id.map(|i| i.as_u64()));
            }
        }
        self.session
            .set_sidechain_source(
                InstrumentId::new(instrument_id),
                source.map(InstrumentId::new),
            )
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn set_instrument_volume(&self, instrument_id: u64, volume: f32) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_volume(
                InstrumentId::new(instrument_id),
                synth_core::Gain::new(volume),
            )
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_volume",
            })
    }

    fn set_instrument_pan(&self, instrument_id: u64, pan: f32) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_pan(
                InstrumentId::new(instrument_id),
                synth_core::BipolarValue::new(pan),
            )
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_pan",
            })
    }

    fn set_instrument_mute(&self, instrument_id: u64, muted: bool) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_mute(InstrumentId::new(instrument_id), muted)
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_mute",
            })
    }

    fn set_instrument_solo(&self, instrument_id: u64, solo: bool) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_solo(InstrumentId::new(instrument_id), solo)
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_solo",
            })
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
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_midi_channel",
            })
    }

    fn set_instrument_enabled(
        &self,
        instrument_id: u64,
        enabled: bool,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_enabled(InstrumentId::new(instrument_id), enabled)
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_enabled",
            })
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
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_category",
            })
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
                _ => McpBridgeError::CommandSendFailed {
                    command: "set_parameter",
                },
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
                value,
                display: pd.unit.format(value),
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
        let midi_channel = MidiChannel::from_one_indexed(channel).unwrap_or_else(|| {
            eprintln!("mcp_bridge: invalid MIDI channel {channel}, falling back to CH1");
            MidiChannel::CH1
        });
        if self.session.command_sender().send(EngineCommand::NoteOn {
            note: MidiNote::new(note),
            velocity: Velocity::from_midi(velocity),
            channel: midi_channel,
        }) {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed { command: "note_on" })
        }
    }

    fn note_off(&self, note: u8, channel: u8) -> Result<(), McpBridgeError> {
        let midi_channel = MidiChannel::from_one_indexed(channel).unwrap_or_else(|| {
            eprintln!("mcp_bridge: invalid MIDI channel {channel}, falling back to CH1");
            MidiChannel::CH1
        });
        if self.session.command_sender().send(EngineCommand::NoteOff {
            note: MidiNote::new(note),
            channel: midi_channel,
        }) {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed {
                command: "note_off",
            })
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
                                        ParamValue::SampleId { sample_id } => {
                                            PatchParamValue::SampleId(*sample_id)
                                        }
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
                    // Apply to engine immediately so audio works regardless of
                    // whether the GUI tick happens to consume the queue.
                    let inst_id = self
                        .session
                        .add_instrument(&patch.name)
                        .map_err(|e| McpBridgeError::Other(e.to_string()))?;
                    let _ = self.session.apply_patch(inst_id, &patch);
                    // Queue for GUI so it can update its current-patch label and
                    // patch_editor cache on the next frame; reconcile_with_session
                    // covers the parameter-state sync.
                    if let Ok(mut pending) = self.shared.pending_patch.lock() {
                        *pending = Some((patch, patch_name.clone()));
                    }
                    return Ok(format!(
                        "OK: {patch_name} loaded as instrument {}",
                        inst_id.as_u64()
                    ));
                }
            }
        }

        Err(McpBridgeError::PatchNotFound(name.to_string()))
    }

    fn request_auto_layout(&self) -> Result<String, McpBridgeError> {
        self.shared
            .pending_auto_layout
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok("OK: auto-layout queued — applied on the next Rack-view frame".to_string())
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

        let mut result = Vec::new();
        for &mt in ALL_MODULE_TYPES {
            if let Some(desc) = get_descriptor(mt) {
                result.push(build_module_type_info(mt, &desc));
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
            .map_err(|_| McpBridgeError::CommandSendFailed { command: "connect" })
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
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "disconnect",
            })
    }

    fn clear_graph(&self, instrument_id: u64) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        self.session
            .clear_graph(InstrumentId::new(instrument_id))
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "clear_graph",
            })
    }

    // === Sequencer: Song ===

    fn get_song_info(&self) -> Result<SongInfo, McpBridgeError> {
        let song = self.shared.song.read();
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
            let mut song = self.shared.song.write();
            song.default_tempo = synth_core::Bpm::new(bpm);
        }
        // Also update engine transport tempo
        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetTempo(synth_core::Bpm::new(bpm)))
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "SetTempo",
            });
        }
        Ok(())
    }

    fn set_song_name(&self, name: &str) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        song.name = name.to_string();
        Ok(())
    }

    // === Sequencer: Patterns ===

    fn list_patterns(&self) -> Result<Vec<PatternInfo>, McpBridgeError> {
        let song = self.shared.song.read();
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
        let mut song = self.shared.song.write();
        let duration = synth_sequencer::Duration(beats_to_ticks(length_beats));
        let id = song.create_pattern(duration);
        if let Some(pattern) = song.pattern_mut(id) {
            pattern.name = name.to_string();
        }
        Ok(id.0)
    }

    fn delete_pattern(&self, pattern_id: u32) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let id = synth_sequencer::PatternId::new(pattern_id);
        song.delete_pattern(id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        Ok(())
    }

    // === Sequencer: Notes ===

    fn list_notes(&self, pattern_id: u32) -> Result<Vec<NoteInfo>, McpBridgeError> {
        let song = self.shared.song.read();
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
        let mut song = self.shared.song.write();
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
        let mut song = self.shared.song.write();
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
        let mut song = self.shared.song.write();
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
        let song = self.shared.song.read();
        let mut tracks: Vec<TrackInfo> = song
            .tracks()
            .map(|t| TrackInfo {
                id: t.id.0,
                name: t.name.clone(),
                instrument_id: t.instrument.map(|i| i.0),
                volume: t.volume.as_f32(),
                // Convert normalized (0.0..1.0) to bipolar (-1.0..1.0) for MCP API
                pan: t.pan.as_f32() * 2.0 - 1.0,
                mute: t.mute,
                solo: t.solo,
            })
            .collect();
        tracks.sort_by_key(|t| t.id);
        Ok(tracks)
    }

    fn create_track(&self, name: &str, instrument_id: Option<u16>) -> Result<u16, McpBridgeError> {
        let mut song = self.shared.song.write();
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
        let mut song = self.shared.song.write();
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
        let mut song = self.shared.song.write();
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let tid = synth_sequencer::TrackId(track_id);
        let tick = synth_sequencer::Tick(u64::from(beats_to_ticks(start_beat)));
        song.remove_placement(pid, tid, tick);
        Ok(())
    }

    fn list_arrangement(&self) -> Result<Vec<PlacementInfo>, McpBridgeError> {
        let song = self.shared.song.read();
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
        let mut song = self.shared.song.write();
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let mut items = Vec::with_capacity(notes.len());
        let mut succeeded = 0usize;

        for (i, n) in notes.iter().enumerate() {
            match try_insert_note_into_pattern(pattern, n) {
                Ok(note_id) => {
                    items.push(BatchItemResult {
                        index: i,
                        success: true,
                        id: Some(note_id),
                        error: None,
                    });
                    succeeded += 1;
                }
                Err(err) => {
                    items.push(BatchItemResult {
                        index: i,
                        success: false,
                        id: None,
                        error: Some(err),
                    });
                }
            }
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
        let mut song = self.shared.song.write();
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let mut items = Vec::with_capacity(updates.len());
        let mut succeeded = 0usize;

        for (i, u) in updates.iter().enumerate() {
            let nid = synth_sequencer::NoteId(u.note_id);
            if let Some(note) = pattern.note_mut(nid) {
                if let Some(p) = u.pitch {
                    if let Some(new_pitch) = synth_sequencer::Pitch::new(p) {
                        note.pitch = new_pitch;
                    } else {
                        items.push(BatchItemResult {
                            index: i,
                            success: false,
                            id: None,
                            error: Some(format!("invalid pitch value: {p}")),
                        });
                        continue;
                    }
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
        let mut song = self.shared.song.write();
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        pattern.clear_notes();

        let mut items = Vec::with_capacity(notes.len());
        let mut succeeded = 0usize;

        for (i, n) in notes.iter().enumerate() {
            match try_insert_note_into_pattern(pattern, n) {
                Ok(note_id) => {
                    items.push(BatchItemResult {
                        index: i,
                        success: true,
                        id: Some(note_id),
                        error: None,
                    });
                    succeeded += 1;
                }
                Err(err) => {
                    items.push(BatchItemResult {
                        index: i,
                        success: false,
                        id: None,
                        error: Some(err),
                    });
                }
            }
        }

        Ok(BatchResult {
            total: notes.len(),
            succeeded,
            failed: notes.len() - succeeded,
            items,
        })
    }

    fn clear_pattern(&self, pattern_id: u32) -> Result<usize, McpBridgeError> {
        let mut song = self.shared.song.write();
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
        let mut song = self.shared.song.write();

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
        let mut song = self.shared.song.write();

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
        let mut song = self.shared.song.write();

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
        let mut song = self.shared.song.write();

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
        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetTempo(synth_core::Bpm::new(tempo)))
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "SetTempo",
            });
        }

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
            Err(McpBridgeError::CommandSendFailed { command: "play" })
        }
    }

    fn seq_stop(&self) -> Result<(), McpBridgeError> {
        if self.session.command_sender().send(EngineCommand::Stop) {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed { command: "stop" })
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
            Err(McpBridgeError::CommandSendFailed { command: "seek" })
        }
    }

    fn build_instrument(
        &self,
        spec: &BridgeInstrumentDef,
    ) -> Result<BuildInstrumentResult, McpBridgeError> {
        use crate::patch::ParamValue;

        // 1. Create or reuse instrument
        let mut errors = Vec::new();
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
            if let Err(e) = self.session.rename_instrument(iid, &spec.name) {
                errors.push(format!("rename: {e}"));
            }
            iid
        } else {
            self.session
                .add_instrument(&spec.name)
                .map_err(|e| McpBridgeError::Other(e.to_string()))?
        };

        // 2. Set optional instrument params
        if let Some(ch) = spec.midi_channel
            && let Err(e) = self.session.set_instrument_midi_channel(
                inst_id,
                MidiChannel::from_one_indexed(ch).unwrap_or(MidiChannel::CH1),
            )
        {
            errors.push(format!("midi_channel: {e}"));
        }
        if let Some(vol) = spec.volume
            && let Err(e) = self
                .session
                .set_instrument_volume(inst_id, synth_core::Gain::new(vol))
        {
            errors.push(format!("volume: {e}"));
        }
        if let Some(pan) = spec.pan
            && let Err(e) = self
                .session
                .set_instrument_pan(inst_id, synth_core::BipolarValue::new(pan))
        {
            errors.push(format!("pan: {e}"));
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

        let mut song_w = self.shared.song.write();
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
        let song = self.shared.song.read();
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
        let song = self.shared.song.read();
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
        let mut song = self.shared.song.write();
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
        let mut song = self.shared.song.write();
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
        let mut song = self.shared.song.write();
        let tid = synth_sequencer::TrackId(track_id);
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.volume = synth_core::NormalizedValue::new(volume);
        Ok(())
    }

    fn set_track_pan(&self, track_id: u16, pan: f32) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let tid = synth_sequencer::TrackId(track_id);
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        // Convert bipolar (-1.0..1.0) to normalized (0.0..1.0) for internal storage
        track.pan = synth_core::NormalizedValue::new((pan + 1.0) * 0.5);
        Ok(())
    }

    fn set_track_mute(&self, track_id: u16, muted: bool) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let tid = synth_sequencer::TrackId(track_id);
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.mute = muted;
        Ok(())
    }

    fn set_track_solo(&self, track_id: u16, solo: bool) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
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
        let mut song = self.shared.song.write();
        let tid = synth_sequencer::TrackId(track_id);
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.instrument = instrument_id.map(synth_sequencer::SeqInstrumentId);
        Ok(())
    }

    fn rename_track(&self, track_id: u16, name: &str) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let tid = synth_sequencer::TrackId(track_id);
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.name = name.to_string();
        Ok(())
    }

    fn delete_track(&self, track_id: u16) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let tid = synth_sequencer::TrackId(track_id);
        song.delete_track(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        Ok(())
    }

    // === Pattern management ===

    fn rename_pattern(&self, pattern_id: u32, name: &str) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        pattern.name = name.to_string();
        Ok(())
    }

    fn set_pattern_length(&self, pattern_id: u32, length_beats: f32) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let pid = synth_sequencer::PatternId::new(pattern_id);
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        pattern.length = synth_sequencer::Duration(beats_to_ticks(length_beats));
        Ok(())
    }

    fn duplicate_pattern(&self, pattern_id: u32) -> Result<u32, McpBridgeError> {
        let mut song = self.shared.song.write();
        let pid = synth_sequencer::PatternId::new(pattern_id);
        song.duplicate_pattern(pid)
            .map(|new_id| new_id.0)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))
    }

    // === Song metadata ===

    fn set_song_author(&self, author: &str) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        song.author = author.to_string();
        Ok(())
    }

    fn set_song_time_signature(
        &self,
        numerator: u8,
        denominator: u8,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
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
            let mut song = self.shared.song.write();
            song.remove_unused()
        };

        // Remove instruments not referenced by remaining tracks/notes
        let mut removed_instruments = Vec::new();
        let snapshots = self.session.list_instruments();
        for snap in &snapshots {
            #[allow(clippy::cast_possible_truncation)]
            if !used_instrument_ids
                .contains(&synth_sequencer::SeqInstrumentId(snap.id.as_u64() as u16))
                && self.session.remove_instrument(snap.id).is_ok()
            {
                removed_instruments.push(snap.name.clone());
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
            &self.sample_library,
            InstrumentId::new(instrument_id),
            MidiNote::new(note),
            Velocity::from_midi(velocity),
            duration_ms,
            tail_ms,
        )
    }

    fn analyze_note(
        &self,
        instrument_id: u64,
        note: u8,
        velocity: u8,
        duration_ms: u32,
        tail_ms: u32,
        expected_note: Option<u8>,
    ) -> Result<synth_mcp::types::AnalyzeNoteResult, McpBridgeError> {
        analyze_rendered_note(
            &self.session,
            &self.sample_library,
            instrument_id,
            note,
            velocity,
            duration_ms,
            tail_ms,
            expected_note,
        )
    }

    fn analyze_harmony(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        grouping_ticks: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<u16>>,
    ) -> Result<AnalyzeHarmonyResult, McpBridgeError> {
        analyze_song_harmony(
            &self.session,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            grouping_ticks,
            exclude_drums,
            exclude_track_ids,
        )
    }

    fn analyze_pattern(&self, pattern_id: u32) -> Result<AnalyzePatternResult, McpBridgeError> {
        analyze_pattern_impl(&self.shared, pattern_id)
    }

    fn analyze_drum_groove(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
    ) -> Result<synth_mcp::types::AnalyzeDrumGrooveResult, McpBridgeError> {
        analyze_drum_groove_impl(
            &self.session,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
        )
    }

    fn analyze_bass_drum_lock(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        onset_tolerance_ticks: Option<u32>,
    ) -> Result<synth_mcp::types::AnalyzeBassDrumLockResult, McpBridgeError> {
        analyze_bass_drum_lock_impl(
            &self.session,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            onset_tolerance_ticks,
        )
    }

    fn analyze_harmonic_function(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        grouping_ticks: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<u16>>,
    ) -> Result<synth_mcp::types::AnalyzeHarmonicFunctionResult, McpBridgeError> {
        analyze_harmonic_function_impl(
            &self.session,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            grouping_ticks,
            exclude_drums,
            exclude_track_ids,
        )
    }

    fn analyze_arrangement(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        similarity_threshold: Option<f32>,
        section_min_bars: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<u16>>,
    ) -> Result<synth_mcp::types::AnalyzeArrangementResult, McpBridgeError> {
        analyze_arrangement_impl(
            &self.session,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            similarity_threshold,
            section_min_bars,
            exclude_drums,
            exclude_track_ids,
        )
    }

    fn analyze_form_map(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        similarity_threshold: Option<f32>,
        section_min_bars: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<u16>>,
    ) -> Result<synth_mcp::types::AnalyzeFormMapResult, McpBridgeError> {
        analyze_form_map_impl(
            &self.session,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            similarity_threshold,
            section_min_bars,
            exclude_drums,
            exclude_track_ids,
        )
    }

    fn find_motifs(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        min_interval_length: Option<u8>,
        max_interval_length: Option<u8>,
        min_count: Option<u32>,
        top_n: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<u16>>,
    ) -> Result<synth_mcp::types::FindMotifsResult, McpBridgeError> {
        find_motifs_impl(
            &self.session,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            min_interval_length,
            max_interval_length,
            min_count,
            top_n,
            exclude_drums,
            exclude_track_ids,
        )
    }

    fn analyze_hook_strength(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        min_interval_length: Option<u8>,
        min_count: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<u16>>,
    ) -> Result<synth_mcp::types::AnalyzeHookStrengthResult, McpBridgeError> {
        analyze_hook_strength_impl(
            &self.session,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            min_interval_length,
            min_count,
            exclude_drums,
            exclude_track_ids,
        )
    }

    fn analyze_mix_bus(
        &self,
        duration_seconds: f32,
        start_tick: Option<u64>,
    ) -> Result<AnalyzeMixBusResult, McpBridgeError> {
        analyze_mix_bus_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            duration_seconds,
            start_tick,
        )
    }

    fn analyze_section(
        &self,
        start_tick: u64,
        end_tick: u64,
        include_per_track: Option<bool>,
    ) -> Result<AnalyzeSectionResult, McpBridgeError> {
        analyze_section_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            start_tick,
            end_tick,
            include_per_track,
        )
    }

    fn analyze_masking_matrix(
        &self,
        start_tick: u64,
        end_tick: u64,
    ) -> Result<AnalyzeMaskingMatrixResult, McpBridgeError> {
        analyze_masking_matrix_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            start_tick,
            end_tick,
        )
    }

    fn analyze_instrument_range(
        &self,
        instrument_id: u64,
        low_note: u8,
        high_note: u8,
        step_semitones: Option<u8>,
        velocity: Option<u8>,
        duration_ms: Option<u32>,
        tail_ms: Option<u32>,
    ) -> Result<synth_mcp::types::AnalyzeInstrumentRangeResult, McpBridgeError> {
        analyze_instrument_range_impl(
            &self.session,
            &self.sample_library,
            instrument_id,
            low_note,
            high_note,
            step_semitones,
            velocity,
            duration_ms,
            tail_ms,
        )
    }

    fn analyze_velocity_response(
        &self,
        instrument_id: u64,
        note: u8,
        velocity_low: Option<u8>,
        velocity_high: Option<u8>,
        velocity_step: Option<u8>,
        duration_ms: Option<u32>,
        tail_ms: Option<u32>,
    ) -> Result<synth_mcp::types::AnalyzeVelocityResponseResult, McpBridgeError> {
        analyze_velocity_response_impl(
            &self.session,
            &self.sample_library,
            instrument_id,
            note,
            velocity_low,
            velocity_high,
            velocity_step,
            duration_ms,
            tail_ms,
        )
    }

    fn generate_chord(
        &self,
        symbol: &str,
        octave: i32,
        voicing: Option<&str>,
    ) -> Result<synth_mcp::types::GenerateChordResult, McpBridgeError> {
        generate_chord_impl(symbol, octave, voicing)
    }

    fn transpose_notes(
        &self,
        pattern_id: u32,
        semitones: i32,
        scale_tonic: Option<u8>,
        scale_name: Option<&str>,
        tie_break: Option<&str>,
    ) -> Result<synth_mcp::types::TransposeNotesResult, McpBridgeError> {
        transpose_notes_impl(
            &self.shared,
            pattern_id,
            semitones,
            scale_tonic,
            scale_name,
            tie_break,
        )
    }

    fn quantize_notes_to_scale(
        &self,
        pattern_id: u32,
        scale_tonic: u8,
        scale_name: &str,
        tie_break: Option<&str>,
    ) -> Result<synth_mcp::types::QuantizeNotesToScaleResult, McpBridgeError> {
        quantize_notes_to_scale_impl(&self.shared, pattern_id, scale_tonic, scale_name, tie_break)
    }

    fn quantize_notes_to_grid(
        &self,
        pattern_id: u32,
        grid_ticks: u32,
        strength: Option<f32>,
        swing: Option<f32>,
        humanize_ticks: Option<u32>,
        humanize_seed: Option<u64>,
    ) -> Result<synth_mcp::types::QuantizeNotesToGridResult, McpBridgeError> {
        quantize_notes_to_grid_impl(
            &self.shared,
            pattern_id,
            grid_ticks,
            strength,
            swing,
            humanize_ticks,
            humanize_seed,
        )
    }

    // === AWE (Acoustic World Engine) ===

    fn get_awe_state(&self) -> Result<AweStateInfo, McpBridgeError> {
        let state = self
            .shared
            .awe_state
            .lock()
            .map_err(|_| McpBridgeError::Other("AWE state lock poisoned".to_string()))?;
        let mut info = awe_state_to_info(&state);
        if let Ok(desc) = self.shared.awe_description.lock()
            && !desc.is_empty()
        {
            info.description = Some(desc.clone());
        }
        Ok(info)
    }

    fn set_awe_description(&self, description: &str) -> Result<(), McpBridgeError> {
        // Description lives separately from the `AweState` struct so
        // existing literal initializers stay untouched. Metadata only —
        // no engine command needed.
        if let Ok(mut desc) = self.shared.awe_description.lock() {
            desc.clear();
            desc.push_str(description);
        }
        Ok(())
    }

    fn set_awe_enabled(&self, enabled: bool) -> Result<(), McpBridgeError> {
        // Update shared state
        if let Ok(mut state) = self.shared.awe_state.lock() {
            state.enabled = enabled;
        }
        // Queue for GUI consumption
        if let Ok(mut pending) = self.shared.pending_awe_state.lock() {
            let current = self
                .shared
                .awe_state
                .lock()
                .map_err(|_| McpBridgeError::Other("AWE state lock poisoned".to_string()))?
                .clone();
            *pending = Some(current);
        }
        // Send engine command
        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetAweEnabled { enabled })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "SetAweEnabled",
            });
        }
        Ok(())
    }

    fn set_awe_parameter(&self, name: &str, value: f64) -> Result<(), McpBridgeError> {
        use synth_awe::{AweParam, Celsius, Meters, Position3, StretchFactor};
        use synth_core::{BipolarValue, Hertz, Milliseconds, NormalizedValue};

        let v = value as f32;

        /// Validate that `v` is within `[min, max]` for parameter `name`.
        fn check(name: &'static str, v: f32, min: f32, max: f32) -> Result<(), McpBridgeError> {
            if v < min || v > max || v.is_nan() {
                return Err(McpBridgeError::ValueOutOfRange {
                    name,
                    value: v,
                    min,
                    max,
                });
            }
            Ok(())
        }

        /// Validate a position value against room bounds.
        fn check_pos(name: &str, v: f32, min: f32, max: f32) -> Result<(), McpBridgeError> {
            if v < min || v > max || v.is_nan() {
                return Err(McpBridgeError::Other(format!(
                    "{name} value {v:.2} out of range: must be {min:.1}..={max:.1} meters \
                     (within current room bounds with 0.1m margin). \
                     Use get_awe_state to see room dimensions, or set_awe_room_shape to resize."
                )));
            }
            Ok(())
        }

        let param = match name {
            "dry_wet" => {
                check("dry_wet", v, 0.0, 1.0)?;
                AweParam::DryWet(NormalizedValue::new(v))
            }
            "early_late_balance" => {
                check("early_late_balance", v, 0.0, 1.0)?;
                AweParam::EarlyLateBalance(NormalizedValue::new(v))
            }
            "modes_amount" => {
                check("modes_amount", v, 0.0, 1.0)?;
                AweParam::ModesAmount(NormalizedValue::new(v))
            }
            "freq_warp" => {
                check("freq_warp", v, -1.0, 1.0)?;
                AweParam::FreqWarp(BipolarValue::new(v))
            }
            "resonance_boost" => {
                check("resonance_boost", v, 0.0, 1.0)?;
                AweParam::ResonanceBoost(NormalizedValue::new(v))
            }
            "tail_stretch" => {
                check("tail_stretch", v, 0.5, 4.0)?;
                AweParam::TailStretch(StretchFactor::new(v))
            }
            "portal_amount" => {
                check("portal_amount", v, 0.0, 1.0)?;
                AweParam::PortalAmount(NormalizedValue::new(v))
            }
            "pre_delay" => {
                check("pre_delay", v, 0.0, 200.0)?;
                AweParam::PreDelay(Milliseconds::new(v))
            }
            "modulation_depth" => {
                check("modulation_depth", v, 0.0, 1.0)?;
                AweParam::ModulationDepth(NormalizedValue::new(v))
            }
            "modulation_rate" => {
                check("modulation_rate", v, 0.01, 20.0)?;
                AweParam::ModulationRate(Hertz::new(v))
            }
            "air_absorption" => {
                check("air_absorption", v, 0.0, 1.0)?;
                AweParam::AirAbsorption(NormalizedValue::new(v))
            }
            "width" => {
                check("width", v, 0.0, 1.0)?;
                AweParam::Width(NormalizedValue::new(v))
            }
            "high_cut" => {
                check("high_cut", v, 200.0, 20000.0)?;
                AweParam::HighCut(Hertz::new(v))
            }
            "low_cut" => {
                check("low_cut", v, 20.0, 2000.0)?;
                AweParam::LowCut(Hertz::new(v))
            }
            "temperature" => {
                check("temperature", v, -40.0, 60.0)?;
                AweParam::Temperature(Celsius::new(v))
            }
            "source_x" | "source_y" | "listener_x" | "listener_y" => {
                // Read current state, build param, and update state in a single lock
                // scope to avoid a race between two separate locks.
                let mut state =
                    self.shared.awe_state.lock().map_err(|_| {
                        McpBridgeError::Other("AWE state lock poisoned".to_string())
                    })?;
                let room = state.room;
                let max_x = room.length().as_f32();
                let max_y = room.width().as_f32();
                let margin = 0.1;
                let p = match name {
                    "source_x" => {
                        check_pos("source_x", v, margin, max_x - margin)?;
                        AweParam::SourcePos(Position3::new(
                            Meters::new(v),
                            state.snapshot.source_pos.y(),
                            state.snapshot.source_pos.z(),
                        ))
                    }
                    "source_y" => {
                        check_pos("source_y", v, margin, max_y - margin)?;
                        AweParam::SourcePos(Position3::new(
                            state.snapshot.source_pos.x(),
                            Meters::new(v),
                            state.snapshot.source_pos.z(),
                        ))
                    }
                    "listener_x" => {
                        check_pos("listener_x", v, margin, max_x - margin)?;
                        AweParam::ListenerPos(Position3::new(
                            Meters::new(v),
                            state.snapshot.listener_pos.y(),
                            state.snapshot.listener_pos.z(),
                        ))
                    }
                    "listener_y" => {
                        check_pos("listener_y", v, margin, max_y - margin)?;
                        AweParam::ListenerPos(Position3::new(
                            state.snapshot.listener_pos.x(),
                            Meters::new(v),
                            state.snapshot.listener_pos.z(),
                        ))
                    }
                    _ => unreachable!(),
                };
                apply_awe_param_to_state(&mut state, &p);
                p
            }
            _ => return Err(McpBridgeError::InvalidAweParameter(name.to_string())),
        };

        // Update shared state snapshot (skip for position params — already updated above)
        if !matches!(param, AweParam::SourcePos(_) | AweParam::ListenerPos(_))
            && let Ok(mut state) = self.shared.awe_state.lock()
        {
            apply_awe_param_to_state(&mut state, &param);
        }

        // Queue for GUI
        queue_awe_for_gui(&self.shared)?;

        // Send engine command
        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetAweParameter { param })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "SetAweParameter",
            });
        }
        Ok(())
    }

    fn set_awe_room_shape(&self, shape: &str, dimensions: &[f32]) -> Result<(), McpBridgeError> {
        let room = parse_room_shape(shape, dimensions)?;

        // Update shared state
        if let Ok(mut state) = self.shared.awe_state.lock() {
            state.room = room;
        }

        // Queue for GUI
        queue_awe_for_gui(&self.shared)?;

        // Send engine command
        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetAweParameter {
                param: synth_awe::AweParam::RoomShape(room),
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "SetAweParameter",
            });
        }
        Ok(())
    }

    fn set_awe_material(&self, material: &str) -> Result<(), McpBridgeError> {
        let mat = parse_material(material)?;

        // Update shared state
        if let Ok(mut state) = self.shared.awe_state.lock() {
            state.material = mat;
        }

        // Queue for GUI
        queue_awe_for_gui(&self.shared)?;

        // Send engine command
        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetAweParameter {
                param: synth_awe::AweParam::Material(mat),
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "SetAweParameter",
            });
        }
        Ok(())
    }

    fn set_awe_preset(&self, name: &str) -> Result<AweStateInfo, McpBridgeError> {
        let presets = synth_awe::awe_presets();
        let preset = presets
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| McpBridgeError::AwePresetNotFound(name.to_string()))?;

        let state = preset.state.clone();

        // Update shared state
        if let Ok(mut shared) = self.shared.awe_state.lock() {
            *shared = state.clone();
        }

        // Queue for GUI
        queue_awe_for_gui(&self.shared)?;

        // Send engine commands (same sequence as GUI)
        let sender = self.session.command_sender();
        let mut failed = 0u32;
        if !sender.send(EngineCommand::SetAweEnabled {
            enabled: state.enabled,
        }) {
            failed += 1;
        }
        if !sender.send(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::RoomShape(state.room),
        }) {
            failed += 1;
        }
        if !sender.send(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::Material(state.material),
        }) {
            failed += 1;
        }
        if !sender.send(EngineCommand::SetAweState {
            snapshot: state.snapshot,
        }) {
            failed += 1;
        }
        if !sender.send(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::SpatialEnabled(state.spatial_enabled),
        }) {
            failed += 1;
        }
        if !sender.send(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::NoteMapping(state.note_mapping),
        }) {
            failed += 1;
        }
        if failed > 0 {
            return Err(McpBridgeError::Other(format!(
                "set_awe_preset: {failed} command(s) failed to send (queue full)"
            )));
        }

        Ok(awe_state_to_info(&state))
    }

    fn list_awe_presets(&self) -> Result<Vec<AwePresetInfo>, McpBridgeError> {
        let presets = synth_awe::awe_presets();
        Ok(presets
            .iter()
            .map(|p| AwePresetInfo {
                name: p.name.to_string(),
                description: p.description.to_string(),
            })
            .collect())
    }

    fn set_awe_lfo(
        &self,
        index: u8,
        rate: f32,
        amount: f32,
        target: &str,
    ) -> Result<(), McpBridgeError> {
        use synth_awe::AweParam;
        use synth_core::{Hertz, NormalizedValue};

        if !(1..=4).contains(&index) {
            return Err(McpBridgeError::InvalidLfoIndex(index));
        }
        let lfo_target = parse_lfo_target(target)?;
        if !(0.01..=20.0).contains(&rate) {
            return Err(McpBridgeError::ValueOutOfRange {
                name: "rate",
                value: rate,
                min: 0.01,
                max: 20.0,
            });
        }
        if !(0.0..=1.0).contains(&amount) {
            return Err(McpBridgeError::ValueOutOfRange {
                name: "amount",
                value: amount,
                min: 0.0,
                max: 1.0,
            });
        }
        let rate_hz = Hertz::new(rate);
        let amt = NormalizedValue::new(amount);

        // Update shared state
        if let Ok(mut state) = self.shared.awe_state.lock() {
            let lfo = match index {
                1 => &mut state.snapshot.lfo1,
                2 => &mut state.snapshot.lfo2,
                3 => &mut state.snapshot.lfo3,
                4 => &mut state.snapshot.lfo4,
                _ => unreachable!(),
            };
            lfo.rate = rate_hz;
            lfo.amount = amt;
            lfo.target = lfo_target;
        }

        // Queue for GUI
        queue_awe_for_gui(&self.shared)?;

        // Send engine commands for rate, amount, target
        let sender = self.session.command_sender();
        let mut failed = 0u32;
        let (rate_param, amt_param, target_param) = match index {
            1 => (
                AweParam::Lfo1Rate(rate_hz),
                AweParam::Lfo1Amount(amt),
                AweParam::Lfo1Target(lfo_target),
            ),
            2 => (
                AweParam::Lfo2Rate(rate_hz),
                AweParam::Lfo2Amount(amt),
                AweParam::Lfo2Target(lfo_target),
            ),
            3 => (
                AweParam::Lfo3Rate(rate_hz),
                AweParam::Lfo3Amount(amt),
                AweParam::Lfo3Target(lfo_target),
            ),
            4 => (
                AweParam::Lfo4Rate(rate_hz),
                AweParam::Lfo4Amount(amt),
                AweParam::Lfo4Target(lfo_target),
            ),
            _ => unreachable!(),
        };
        for param in [rate_param, amt_param, target_param] {
            if !sender.send(EngineCommand::SetAweParameter { param }) {
                failed += 1;
            }
        }
        if failed > 0 {
            return Err(McpBridgeError::Other(format!(
                "set_awe_lfo: {failed} command(s) failed to send (queue full)"
            )));
        }

        Ok(())
    }

    // === Sample library ===

    fn list_samples(
        &self,
        filter: Option<&str>,
    ) -> Result<Vec<synth_mcp::types::SampleInfo>, McpBridgeError> {
        let lib = self
            .sample_library
            .read()
            .map_err(|_| McpBridgeError::Other("Sample library lock poisoned".to_string()))?;
        let metas = lib.list();
        let filter_lower = filter.map(|f| f.to_lowercase());
        Ok(metas
            .into_iter()
            .filter(|m| {
                filter_lower
                    .as_ref()
                    .is_none_or(|f| m.name.to_lowercase().contains(f))
            })
            .map(meta_to_sample_info)
            .collect())
    }

    fn import_sample(
        &self,
        path: &str,
        name: Option<&str>,
        root_note: Option<u8>,
    ) -> Result<synth_mcp::types::SampleInfo, McpBridgeError> {
        let file_path = std::path::Path::new(path);
        if !file_path.exists() {
            return Err(McpBridgeError::Other(format!("File not found: {path}")));
        }
        let target_rate = synth_core::audio::SampleRate::DVD_QUALITY;
        let mut sample = synth_sampler::load_wav(file_path, target_rate)
            .map_err(|e| McpBridgeError::Other(format!("WAV load error: {e}")))?;
        if let Some(n) = name {
            sample.meta.name = n.to_string();
        }
        if let Some(note) = root_note {
            sample.meta.root_note = Some(synth_core::MidiNote(note));
        }
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Sample library lock poisoned".to_string()))?;
        let id = lib.add(sample);
        let meta = lib.get_meta(id).ok_or_else(|| {
            McpBridgeError::Other("Failed to retrieve imported sample".to_string())
        })?;
        Ok(meta_to_sample_info(meta))
    }

    fn delete_sample(&self, id: u64) -> Result<(), McpBridgeError> {
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Sample library lock poisoned".to_string()))?;
        let sample_id = synth_sampler::SampleId::new(id);
        lib.remove(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
        Ok(())
    }

    fn rename_sample(&self, id: u64, name: &str) -> Result<(), McpBridgeError> {
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Sample library lock poisoned".to_string()))?;
        let sample_id = synth_sampler::SampleId::new(id);
        let meta = lib
            .get_meta(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?
            .clone();
        let mut updated = meta;
        updated.name = name.to_string();
        lib.update_meta(sample_id, updated);
        Ok(())
    }

    fn set_sample_root_note(&self, id: u64, note: u8) -> Result<(), McpBridgeError> {
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Sample library lock poisoned".to_string()))?;
        let sample_id = synth_sampler::SampleId::new(id);
        let meta = lib
            .get_meta(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?
            .clone();
        let mut updated = meta;
        updated.root_note = Some(synth_core::MidiNote(note));
        lib.update_meta(sample_id, updated);
        Ok(())
    }

    fn normalize_sample(&self, id: u64) -> Result<(), McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let lib = self
            .sample_library
            .read()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        let sample = lib
            .get(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
        let peak = sample.data.iter().fold(0.0_f32, |a, &s| a.max(s.abs()));
        if peak <= 0.0 || (peak - 1.0).abs() < 1e-6 {
            return Ok(());
        }
        let gain = 1.0 / peak;
        let normalized: std::sync::Arc<[f32]> = sample
            .data
            .iter()
            .map(|&s| s * gain)
            .collect::<Vec<_>>()
            .into();
        drop(lib);
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        lib.replace_data(sample_id, normalized);
        Ok(())
    }

    fn reverse_sample(&self, id: u64) -> Result<(), McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let lib = self
            .sample_library
            .read()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        let sample = lib
            .get(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
        let channels = sample.meta.channels.count() as usize;
        let frame_count = sample.meta.frame_count.as_usize();
        let mut reversed = vec![0.0_f32; sample.data.len()];
        for frame in 0..frame_count {
            let src = frame_count - 1 - frame;
            for ch in 0..channels {
                reversed[frame * channels + ch] = sample.data[src * channels + ch];
            }
        }
        let data: std::sync::Arc<[f32]> = reversed.into();
        drop(lib);
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        lib.replace_data(sample_id, data);
        Ok(())
    }

    fn trim_sample_silence(&self, id: u64) -> Result<(), McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let threshold = 0.01_f32;
        let lib = self
            .sample_library
            .read()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        let sample = lib
            .get(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
        let channels = sample.meta.channels.count() as usize;
        let frame_count = sample.meta.frame_count.as_usize();
        let mut start = 0;
        for frame in 0..frame_count {
            let mut peak = 0.0_f32;
            for ch in 0..channels {
                peak = peak.max(sample.data[frame * channels + ch].abs());
            }
            if peak > threshold {
                start = frame;
                break;
            }
        }
        let mut end = frame_count;
        for frame in (0..frame_count).rev() {
            let mut peak = 0.0_f32;
            for ch in 0..channels {
                peak = peak.max(sample.data[frame * channels + ch].abs());
            }
            if peak > threshold {
                end = frame + 1;
                break;
            }
        }
        drop(lib);
        if start < end {
            let mut lib = self
                .sample_library
                .write()
                .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
            lib.update_crop(
                sample_id,
                Some(synth_sampler::CropRegion {
                    start: synth_sampler::FrameIndex::new(start),
                    end: synth_sampler::FrameIndex::new(end),
                }),
            );
        }
        Ok(())
    }

    fn get_sample_info(
        &self,
        id: u64,
    ) -> Result<synth_mcp::types::DetailedSampleInfo, McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let lib = self
            .sample_library
            .read()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        let sample = lib
            .get(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;

        let data = &sample.data;
        let len = data.len();
        let (peak_level, rms_level, dc_offset) = if len == 0 {
            (0.0, 0.0, 0.0)
        } else {
            let (peak, sq_sum, dc_sum) = data
                .iter()
                .fold((0.0_f32, 0.0_f32, 0.0_f32), |(peak, sq, dc), &s| {
                    (peak.max(s.abs()), sq + s * s, dc + s)
                });
            let rms = (sq_sum / len as f32).sqrt();
            let dc = dc_sum / len as f32;
            (peak, rms, dc)
        };
        let memory_bytes = len * std::mem::size_of::<f32>();
        let sr = f64::from(sample.meta.sample_rate.0);

        let (loop_start_seconds, loop_end_seconds) = match &sample.meta.loop_region {
            Some(region) => (
                Some(region.start.0 as f64 / sr),
                Some(region.end.0 as f64 / sr),
            ),
            None => (None, None),
        };

        let (crop_start_seconds, crop_end_seconds) = match &sample.meta.crop {
            Some(region) => (
                Some(region.start.0 as f64 / sr),
                Some(region.end.0 as f64 / sr),
            ),
            None => (None, None),
        };

        let info = meta_to_sample_info(&sample.meta);
        Ok(synth_mcp::types::DetailedSampleInfo {
            info,
            peak_level,
            rms_level,
            dc_offset,
            memory_bytes,
            loop_start_seconds,
            loop_end_seconds,
            crop_start_seconds,
            crop_end_seconds,
        })
    }

    fn duplicate_sample(&self, id: u64) -> Result<synth_mcp::types::SampleInfo, McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let (cloned_data, cloned_meta) = {
            let lib = self
                .sample_library
                .read()
                .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
            let sample = lib
                .get(sample_id)
                .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
            (Arc::clone(&sample.data), sample.meta.clone())
        };

        let new_meta = synth_sampler::SampleMeta {
            id: synth_sampler::SampleId::new(0),
            name: format!("{} (copy)", cloned_meta.name),
            ..cloned_meta
        };
        let new_sample = synth_sampler::Sample::new(new_meta, cloned_data);

        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        let new_id = lib.add(new_sample);
        let new_meta = lib.get_meta(new_id).ok_or_else(|| {
            McpBridgeError::Other("Failed to retrieve duplicated sample".to_string())
        })?;
        Ok(meta_to_sample_info(new_meta))
    }

    fn set_sample_loop(
        &self,
        id: u64,
        enabled: bool,
        start_seconds: Option<f64>,
        end_seconds: Option<f64>,
        crossfade_ms: Option<f64>,
    ) -> Result<(), McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        if enabled {
            let start_s = start_seconds.ok_or_else(|| {
                McpBridgeError::Other("start_seconds required when enabled".to_string())
            })?;
            let end_s = end_seconds.ok_or_else(|| {
                McpBridgeError::Other("end_seconds required when enabled".to_string())
            })?;
            let sample = lib
                .get(sample_id)
                .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
            let sr = f64::from(sample.meta.sample_rate.0);
            let start_frame = (start_s * sr) as usize;
            let end_frame = (end_s * sr) as usize;
            let crossfade_frames = crossfade_ms
                .map(|ms| (ms / 1000.0 * sr) as usize)
                .unwrap_or(0);
            lib.update_loop(
                sample_id,
                Some(synth_sampler::LoopRegion {
                    start: synth_sampler::FrameIndex::new(start_frame),
                    end: synth_sampler::FrameIndex::new(end_frame),
                    crossfade: SampleCount::new(crossfade_frames),
                }),
            );
        } else {
            lib.update_loop(sample_id, None);
        }
        Ok(())
    }

    fn set_sample_crop(
        &self,
        id: u64,
        start_seconds: Option<f64>,
        end_seconds: Option<f64>,
    ) -> Result<(), McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        if start_seconds.is_none() && end_seconds.is_none() {
            lib.update_crop(sample_id, None);
        } else {
            let sample = lib
                .get(sample_id)
                .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
            let sr = f64::from(sample.meta.sample_rate.0);
            let start_frame = start_seconds.map(|s| (s * sr) as usize).unwrap_or(0);
            let end_frame = end_seconds
                .map(|s| (s * sr) as usize)
                .unwrap_or(sample.meta.frame_count.as_usize());
            lib.update_crop(
                sample_id,
                Some(synth_sampler::CropRegion {
                    start: synth_sampler::FrameIndex::new(start_frame),
                    end: synth_sampler::FrameIndex::new(end_frame),
                }),
            );
        }
        Ok(())
    }

    fn export_sample(
        &self,
        id: u64,
        path: &str,
        bit_depth: Option<u8>,
    ) -> Result<(), McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let lib = self
            .sample_library
            .read()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        let sample = lib
            .get(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
        let depth = match bit_depth {
            Some(24) => synth_sampler::BitDepth::Int24,
            Some(32) => synth_sampler::BitDepth::Float32,
            _ => synth_sampler::BitDepth::Int16,
        };
        synth_sampler::save_wav(sample, std::path::Path::new(path), depth)
            .map_err(|e| McpBridgeError::Other(format!("Export failed: {e}")))?;
        Ok(())
    }

    // === Sampler module control ===

    fn assign_sample_to_module(
        &self,
        instrument_id: u64,
        module_id: &str,
        sample_id: u64,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let inst_id = InstrumentId::new(instrument_id);
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;

        // Get sample data from library
        let lib = self
            .sample_library
            .read()
            .map_err(|_| McpBridgeError::Other("Sample library lock poisoned".to_string()))?;
        let sampler_sample_id = synth_sampler::SampleId::new(sample_id);
        let sample = lib
            .get(sampler_sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {sample_id}")))?;
        let data = Arc::clone(&sample.data);
        let channels = sample.meta.channels;
        let frame_count = sample.meta.frame_count.as_usize();
        let root_note = sample.meta.root_note.unwrap_or(MidiNote::new(60));
        drop(lib);

        // Send SampleSelect param
        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetModuleParameter {
                instrument_id: Some(inst_id),
                module_id: mid,
                param: Param::Sampler(synth_core::SamplerParam::SampleSelect(
                    synth_core::SampleId(sample_id),
                )),
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "assign_sample_select",
            });
        }

        // Send the actual audio data
        if !self
            .session
            .command_sender()
            .send(EngineCommand::LoadSampleData {
                instrument_id: inst_id,
                module_id: mid,
                data,
                channels,
                frame_count,
                root_note,
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "load_sample_data",
            });
        }

        Ok(())
    }

    fn get_sampler_state(
        &self,
        instrument_id: u64,
        module_id: &str,
    ) -> Result<synth_mcp::types::SamplerStateInfo, McpBridgeError> {
        use synth_core::{Param, PlayDirection, SamplerParam, SamplerPlayMode};

        self.validate_instrument(instrument_id)?;

        // `Param::as_f32()` returns 0.0 for `SamplerParam::SampleSelect` (the
        // sample id doesn't fit a slider), so the f32 path silently loses it.
        // Read the typed enum from the snapshot instead.
        let inst_id = InstrumentId::new(instrument_id);
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;
        let snapshot = self
            .session
            .state()
            .shared_graph
            .get_module(inst_id, mid)
            .ok_or_else(|| McpBridgeError::ModuleNotFound(module_id.to_string()))?;

        let mut sample_id: Option<synth_sampler::SampleId> = None;
        let mut pitch_tracking = true;
        let mut level: f32 = 0.0;
        let mut play_mode = SamplerPlayMode::OneShot;
        let mut direction = PlayDirection::Forward;
        let mut velocity_sensitivity: f32 = 1.0;
        let mut fine_tune: f32 = 0.0;
        let mut start_offset: f32 = 0.0;
        for param in &snapshot.parameters {
            if let Param::Sampler(sp) = param {
                match sp {
                    SamplerParam::SampleSelect(sid) if sid.0 != 0 => {
                        sample_id = Some(synth_sampler::SampleId::new(sid.0));
                    }
                    SamplerParam::SampleSelect(_) => {}
                    SamplerParam::PitchTracking(b) => pitch_tracking = *b,
                    SamplerParam::Level(g) => level = g.as_f32(),
                    SamplerParam::PlayMode(m) => play_mode = *m,
                    SamplerParam::Direction(d) => direction = *d,
                    SamplerParam::VelocitySensitivity(v) => velocity_sensitivity = v.as_f32(),
                    SamplerParam::FineTune(c) => fine_tune = c.0,
                    SamplerParam::StartOffset(v) => start_offset = v.as_f32(),
                }
            }
        }

        let play_mode_str = match play_mode {
            SamplerPlayMode::OneShot => "one_shot",
            SamplerPlayMode::Sustain => "sustain",
            SamplerPlayMode::Loop => "loop",
        }
        .to_string();

        let direction_str = match direction {
            PlayDirection::Forward => "forward",
            PlayDirection::Reverse => "reverse",
            PlayDirection::PingPong => "ping_pong",
        }
        .to_string();

        let sample_name = if let Some(id) = sample_id {
            let lib = self
                .sample_library
                .read()
                .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
            lib.get_meta(id).map(|m| m.name.clone()).unwrap_or_default()
        } else {
            String::new()
        };

        Ok(synth_mcp::types::SamplerStateInfo {
            sample_id: sample_id.map_or(0, |id| id.0),
            sample_name,
            pitch_tracking,
            level,
            play_mode: play_mode_str,
            direction: direction_str,
            velocity_sensitivity,
            fine_tune,
            start_offset,
        })
    }

    fn set_sampler_parameter(
        &self,
        instrument_id: u64,
        module_id: &str,
        param_name: &str,
        value: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let inst_id = InstrumentId::new(instrument_id);
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;

        let param = match param_name {
            "pitch_tracking" | "pitch_track" => {
                let b = value.parse::<bool>().map_err(|_| {
                    McpBridgeError::Other(format!(
                        "Invalid boolean value '{value}', expected 'true' or 'false'"
                    ))
                })?;
                synth_core::SamplerParam::PitchTracking(b)
            }
            "level" => {
                let v = value
                    .parse::<f32>()
                    .map_err(|_| McpBridgeError::Other(format!("Invalid float value '{value}'")))?;
                synth_core::SamplerParam::Level(synth_core::Gain::new(v))
            }
            "play_mode" => {
                let mode = match value {
                    "one_shot" => synth_core::SamplerPlayMode::OneShot,
                    "sustain" => synth_core::SamplerPlayMode::Sustain,
                    "loop" => synth_core::SamplerPlayMode::Loop,
                    _ => {
                        return Err(McpBridgeError::Other(format!(
                            "Invalid play_mode '{value}', expected one_shot/sustain/loop"
                        )));
                    }
                };
                synth_core::SamplerParam::PlayMode(mode)
            }
            "direction" => {
                let dir = match value {
                    "forward" => synth_core::PlayDirection::Forward,
                    "reverse" => synth_core::PlayDirection::Reverse,
                    "ping_pong" => synth_core::PlayDirection::PingPong,
                    _ => {
                        return Err(McpBridgeError::Other(format!(
                            "Invalid direction '{value}', expected forward/reverse/ping_pong"
                        )));
                    }
                };
                synth_core::SamplerParam::Direction(dir)
            }
            "velocity_sensitivity" | "vel_sens" => {
                let v = value
                    .parse::<f32>()
                    .map_err(|_| McpBridgeError::Other(format!("Invalid float value '{value}'")))?;
                synth_core::SamplerParam::VelocitySensitivity(NormalizedValue::new(v))
            }
            "fine_tune" => {
                let v = value
                    .parse::<f32>()
                    .map_err(|_| McpBridgeError::Other(format!("Invalid float value '{value}'")))?;
                synth_core::SamplerParam::FineTune(synth_core::Cents::new(v))
            }
            "start_offset" => {
                let v = value
                    .parse::<f32>()
                    .map_err(|_| McpBridgeError::Other(format!("Invalid float value '{value}'")))?;
                synth_core::SamplerParam::StartOffset(NormalizedValue::new(v))
            }
            _ => {
                return Err(McpBridgeError::ParameterNotFound(format!(
                    "Unknown sampler parameter '{param_name}'. Available: pitch_tracking, level, \
                     play_mode, direction, velocity_sensitivity, fine_tune, start_offset"
                )));
            }
        };

        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetModuleParameter {
                instrument_id: Some(inst_id),
                module_id: mid,
                param: Param::Sampler(param),
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "set_sampler_parameter",
            });
        }

        Ok(())
    }

    // === Audio input ===

    fn list_input_devices(&self) -> Result<Vec<synth_mcp::types::InputDeviceInfo>, McpBridgeError> {
        // TODO: Stub — needs access to AudioHostTrait to enumerate real input devices.
        // Return empty list until wired up to the audio subsystem.
        Ok(Vec::new())
    }

    fn get_input_state(&self) -> Result<synth_mcp::types::InputStateInfo, McpBridgeError> {
        // TODO: Stub — returns hardcoded idle state. Wire up to real audio input
        // monitoring state once the bridge has access to AudioInputManager.
        Ok(synth_mcp::types::InputStateInfo {
            state: "idle".to_string(),
            peak_level: 0.0,
            recorded_seconds: 0.0,
            is_active: false,
        })
    }

    // === Discovery ===

    fn get_module_type_info(&self, type_key: &str) -> Result<ModuleTypeInfo, McpBridgeError> {
        use crate::module_factory::{ALL_MODULE_TYPES, get_descriptor};

        let mt = synth_core::ModuleType::from_prefix(type_key)
            .ok_or_else(|| McpBridgeError::InvalidModuleType(type_key.to_string()))?;

        if !ALL_MODULE_TYPES.contains(&mt) {
            return Err(McpBridgeError::InvalidModuleType(type_key.to_string()));
        }

        let desc = get_descriptor(mt)
            .ok_or_else(|| McpBridgeError::InvalidModuleType(type_key.to_string()))?;

        Ok(build_module_type_info(mt, &desc))
    }

    fn search_modules(
        &self,
        category: Option<&str>,
        has_input_type: Option<&str>,
        has_output_type: Option<&str>,
        query: Option<&str>,
    ) -> Result<Vec<ModuleTypeInfo>, McpBridgeError> {
        use crate::module_factory::{ALL_MODULE_TYPES, get_descriptor};
        use synth_core::PortDirection;

        let query_lower = query.map(|q| q.to_lowercase());

        let mut result = Vec::new();
        for &mt in ALL_MODULE_TYPES {
            // Category filter — cheap, no descriptor needed
            if let Some(cat) = category {
                let mt_cat = if mt.is_voice_module() {
                    "voice"
                } else if mt.is_effect() {
                    "effect"
                } else {
                    "visualizer"
                };
                if mt_cat != cat {
                    continue;
                }
            }

            let Some(desc) = get_descriptor(mt) else {
                continue;
            };

            // Port type filters — use raw descriptor, skip building full info
            if let Some(input_type) = has_input_type
                && !desc.ports.iter().any(|p| {
                    p.direction == PortDirection::Input && port_type_str(p.port_type) == input_type
                })
            {
                continue;
            }
            if let Some(output_type) = has_output_type
                && !desc.ports.iter().any(|p| {
                    p.direction == PortDirection::Output
                        && port_type_str(p.port_type) == output_type
                })
            {
                continue;
            }

            // Text query — search descriptor strings directly
            if let Some(ref q) = query_lower {
                let name_match = mt.name().to_lowercase().contains(q.as_str());
                let key_match = mt.prefix().to_lowercase().contains(q.as_str());
                let desc_match = desc.description.to_lowercase().contains(q.as_str());
                let param_match = desc
                    .parameters
                    .iter()
                    .any(|p| p.name.to_lowercase().contains(q.as_str()));
                if !name_match && !key_match && !desc_match && !param_match {
                    continue;
                }
            }

            // Only build full ModuleTypeInfo for matches
            result.push(build_module_type_info(mt, &desc));
        }
        Ok(result)
    }

    fn check_connection(
        &self,
        instrument_id: u64,
        from_module: &str,
        from_port: &str,
        to_module: &str,
        to_port: &str,
    ) -> Result<ConnectionCheckResult, McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let inst_id = InstrumentId::new(instrument_id);

        let from_mid: ModuleId = from_module
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(from_module.to_string()))?;
        let to_mid: ModuleId = to_module
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(to_module.to_string()))?;

        let from_desc = self
            .session
            .module_descriptor(inst_id, from_mid)
            .ok_or_else(|| McpBridgeError::ModuleNotFound(from_module.to_string()))?;
        let to_desc = self
            .session
            .module_descriptor(inst_id, to_mid)
            .ok_or_else(|| McpBridgeError::ModuleNotFound(to_module.to_string()))?;

        let from_port_desc = from_desc.ports.iter().find(|p| p.name == from_port);
        let to_port_desc = to_desc.ports.iter().find(|p| p.name == to_port);

        let Some(from_pd) = from_port_desc else {
            let available: Vec<&str> = from_desc.ports.iter().map(|p| p.name.as_str()).collect();
            return Ok(ConnectionCheckResult {
                valid: false,
                from_signal_type: None,
                to_signal_type: None,
                message: format!(
                    "Port '{}' not found on module '{}'.",
                    from_port, from_module
                ),
                hint: Some(format!("Available ports: {}", available.join(", "))),
            });
        };

        let Some(to_pd) = to_port_desc else {
            let available: Vec<&str> = to_desc.ports.iter().map(|p| p.name.as_str()).collect();
            return Ok(ConnectionCheckResult {
                valid: false,
                from_signal_type: None,
                to_signal_type: None,
                message: format!("Port '{}' not found on module '{}'.", to_port, to_module),
                hint: Some(format!("Available ports: {}", available.join(", "))),
            });
        };

        let from_type_str = port_type_str(from_pd.port_type);
        let to_type_str = port_type_str(to_pd.port_type);

        if from_pd.direction != PortDirection::Output {
            let outputs: Vec<&str> = from_desc
                .ports
                .iter()
                .filter(|p| p.direction == PortDirection::Output)
                .map(|p| p.name.as_str())
                .collect();
            return Ok(ConnectionCheckResult {
                valid: false,
                from_signal_type: Some(from_type_str.to_string()),
                to_signal_type: Some(to_type_str.to_string()),
                message: format!(
                    "'{}' on '{}' is an input port, not an output.",
                    from_port, from_module
                ),
                hint: Some(format!(
                    "Output ports on '{}': {}",
                    from_module,
                    if outputs.is_empty() {
                        "(none)".to_string()
                    } else {
                        outputs.join(", ")
                    }
                )),
            });
        };

        if to_pd.direction != PortDirection::Input {
            let inputs: Vec<&str> = to_desc
                .ports
                .iter()
                .filter(|p| p.direction == PortDirection::Input)
                .map(|p| p.name.as_str())
                .collect();
            return Ok(ConnectionCheckResult {
                valid: false,
                from_signal_type: Some(from_type_str.to_string()),
                to_signal_type: Some(to_type_str.to_string()),
                message: format!(
                    "'{}' on '{}' is an output port, not an input.",
                    to_port, to_module
                ),
                hint: Some(format!(
                    "Input ports on '{}': {}",
                    to_module,
                    if inputs.is_empty() {
                        "(none)".to_string()
                    } else {
                        inputs.join(", ")
                    }
                )),
            });
        };

        let compatible = matches!(
            (from_pd.port_type, to_pd.port_type),
            (synth_core::PortType::Audio, synth_core::PortType::Audio)
                | (synth_core::PortType::Audio, synth_core::PortType::Control)
                | (synth_core::PortType::Control, synth_core::PortType::Audio)
                | (synth_core::PortType::Control, synth_core::PortType::Control)
                | (synth_core::PortType::Gate, synth_core::PortType::Gate)
                | (synth_core::PortType::Gate, synth_core::PortType::Control)
                | (synth_core::PortType::Midi, synth_core::PortType::Midi)
        );

        if compatible {
            let note = if from_pd.port_type != to_pd.port_type {
                format!(
                    " (cross-type: {} → {}, signal will be interpreted as {})",
                    from_type_str, to_type_str, to_type_str
                )
            } else {
                String::new()
            };
            Ok(ConnectionCheckResult {
                valid: true,
                from_signal_type: Some(from_type_str.to_string()),
                to_signal_type: Some(to_type_str.to_string()),
                message: format!(
                    "Valid connection: {}:{} → {}:{}{}",
                    from_module, from_port, to_module, to_port, note
                ),
                hint: None,
            })
        } else {
            Ok(ConnectionCheckResult {
                valid: false,
                from_signal_type: Some(from_type_str.to_string()),
                to_signal_type: Some(to_type_str.to_string()),
                message: format!(
                    "Incompatible signal types: {} output → {} input.",
                    from_type_str, to_type_str
                ),
                hint: Some(format!(
                    "{} ports can connect to: {}",
                    from_type_str,
                    compatible_types_hint(from_pd.port_type)
                )),
            })
        }
    }
}

/// Collect module IDs referenced as sources by any enabled Mod Matrix slot.
///
/// The Mod Matrix routes via parameter slots rather than cables, so an LFO,
/// Envelope, or Envelope Follower selected in `Slot N Source` is considered
/// "in use" by the diagnostic even if it has no cable connections.
fn collect_mod_matrix_sources(modules: &[synth_engine::ModuleStateSnapshot]) -> HashSet<String> {
    let mut sources = HashSet::new();
    for module in modules {
        if module.id.module_type != synth_core::ModuleType::ModMatrix {
            continue;
        }

        let mut enabled = [true; synth_core::MAX_MOD_MATRIX_SLOTS];
        let mut slot_source: [Option<ModSource>; synth_core::MAX_MOD_MATRIX_SLOTS] =
            [None; synth_core::MAX_MOD_MATRIX_SLOTS];

        for param in &module.parameters {
            if let Param::ModMatrix(mmx) = param {
                match mmx {
                    ModMatrixParam::SlotEnabled(slot, en) => {
                        if let Some(cell) = enabled.get_mut(*slot as usize) {
                            *cell = *en;
                        }
                    }
                    ModMatrixParam::SlotSource(slot, src) => {
                        if let Some(cell) = slot_source.get_mut(*slot as usize) {
                            *cell = Some(*src);
                        }
                    }
                    _ => {}
                }
            }
        }

        for (slot, source) in slot_source.iter().enumerate() {
            if !enabled[slot] {
                continue;
            }
            let Some(source) = source else { continue };
            if let Some(id) = mod_source_to_module_id(*source) {
                sources.insert(id);
            }
        }
    }
    sources
}

/// Map a `ModSource` variant to the module ID it references, if any.
/// Returns `None` for global sources (Velocity, ModWheel, etc.).
///
/// `ModSource` indexes its source families from 0; `ModuleId` instances start
/// at 1, hence the `+ 1` offset.
fn mod_source_to_module_id(source: ModSource) -> Option<String> {
    use synth_core::ModuleType;
    let typed = match source {
        ModSource::Lfo(i) => ModuleId::new(ModuleType::Lfo, u16::from(i) + 1),
        ModSource::Envelope(i) => ModuleId::new(ModuleType::Envelope, u16::from(i) + 1),
        ModSource::EnvFollower(i) => ModuleId::new(ModuleType::EnvelopeFollower, u16::from(i) + 1),
        ModSource::KineticPos | ModSource::KineticVel | ModSource::KineticAcc => {
            ModuleId::new(ModuleType::KineticModulator, 1)
        }
        _ => return None,
    };
    Some(typed.to_string())
}

fn meta_to_sample_info(meta: &synth_sampler::SampleMeta) -> synth_mcp::types::SampleInfo {
    synth_mcp::types::SampleInfo {
        id: meta.id.0,
        name: meta.name.clone(),
        duration_seconds: meta.duration_seconds(),
        sample_rate: meta.sample_rate.0,
        channels: meta.channels.count(),
        frame_count: meta.frame_count.as_usize(),
        root_note: meta.root_note.map(|n| n.0),
        loop_enabled: meta.loop_region.is_some(),
        has_crop: meta.crop.is_some(),
        source: match &meta.source {
            synth_sampler::SampleSource::Recorded => "recorded".to_string(),
            synth_sampler::SampleSource::Imported { .. } => "imported".to_string(),
            synth_sampler::SampleSource::Generated => "generated".to_string(),
        },
    }
}

// === AWE helper functions ===

/// Queue the current shared AWE state for GUI consumption.
fn queue_awe_for_gui(shared: &crate::mcp_shared::McpSharedState) -> Result<(), McpBridgeError> {
    let current = shared
        .awe_state
        .lock()
        .map_err(|_| McpBridgeError::Other("AWE state lock poisoned".to_string()))?
        .clone();
    if let Ok(mut pending) = shared.pending_awe_state.lock() {
        *pending = Some(current);
    }
    Ok(())
}

/// Convert `AweState` to the serializable `AweStateInfo`.
fn awe_state_to_info(state: &synth_awe::AweState) -> AweStateInfo {
    let room = state.room;
    let snap = &state.snapshot;

    AweStateInfo {
        enabled: state.enabled,
        description: None, // filled in by the bridge after `awe_state_to_info`
        room_shape: room_shape_name(&room),
        room_dimensions: room_dimensions_string(&room),
        room_length: room.length().as_f32(),
        room_width: room.width().as_f32(),
        room_height: room.height().as_f32(),
        room_volume: room.volume().as_f32(),
        material: material_name(&state.material),
        source_position: snap.source_pos.as_f32(),
        listener_position: snap.listener_pos.as_f32(),
        dry_wet: snap.dry_wet.as_f32(),
        early_late_balance: snap.early_late_balance.as_f32(),
        modes_amount: snap.modes_amount.as_f32(),
        freq_warp: snap.freq_warp.as_f32(),
        resonance_boost: snap.resonance_boost.as_f32(),
        tail_stretch: snap.tail_stretch.as_f32(),
        portal_amount: snap.portal_amount.as_f32(),
        pre_delay_ms: snap.pre_delay.as_f32(),
        modulation_depth: snap.modulation_depth.as_f32(),
        modulation_rate: snap.modulation_rate.as_f32(),
        air_absorption: snap.air_absorption.as_f32(),
        width: snap.width.as_f32(),
        high_cut: snap.high_cut.as_f32(),
        low_cut: snap.low_cut.as_f32(),
        temperature: snap.temperature.as_f32(),
        spatial_enabled: state.spatial_enabled,
        note_mapping: mapping_name(&state.note_mapping),
        lfos: vec![
            lfo_to_info(1, &snap.lfo1),
            lfo_to_info(2, &snap.lfo2),
            lfo_to_info(3, &snap.lfo3),
            lfo_to_info(4, &snap.lfo4),
        ],
    }
}

fn lfo_to_info(index: u8, lfo: &synth_awe::params::AweLfoState) -> AweLfoInfo {
    AweLfoInfo {
        index,
        rate: lfo.rate.as_f32(),
        amount: lfo.amount.as_f32(),
        target: lfo_target_name(&lfo.target),
    }
}

fn room_shape_name(shape: &synth_awe::RoomShape) -> String {
    match shape {
        synth_awe::RoomShape::Box { .. } => "Box".to_string(),
        synth_awe::RoomShape::Cylinder { .. } => "Cylinder".to_string(),
        synth_awe::RoomShape::LShape { .. } => "LShape".to_string(),
        synth_awe::RoomShape::Sphere { .. } => "Sphere".to_string(),
        synth_awe::RoomShape::Dome { .. } => "Dome".to_string(),
        synth_awe::RoomShape::Tube { .. } => "Tube".to_string(),
    }
}

fn room_dimensions_string(shape: &synth_awe::RoomShape) -> String {
    match shape {
        synth_awe::RoomShape::Box {
            length,
            width,
            height,
        } => format!(
            "{:.1} x {:.1} x {:.1} m (L x W x H)",
            length.as_f32(),
            width.as_f32(),
            height.as_f32()
        ),
        synth_awe::RoomShape::Cylinder { radius, length } => format!(
            "radius {:.1} m, length {:.1} m",
            radius.as_f32(),
            length.as_f32()
        ),
        synth_awe::RoomShape::LShape {
            length_a,
            width_a,
            length_b,
            width_b,
            height,
        } => format!(
            "section A: {:.1} x {:.1} m, section B: {:.1} x {:.1} m, height {:.1} m",
            length_a.as_f32(),
            width_a.as_f32(),
            length_b.as_f32(),
            width_b.as_f32(),
            height.as_f32()
        ),
        synth_awe::RoomShape::Sphere { radius } => {
            format!("radius {:.1} m", radius.as_f32())
        }
        synth_awe::RoomShape::Dome { radius } => {
            format!("radius {:.1} m", radius.as_f32())
        }
        synth_awe::RoomShape::Tube { radius, length } => format!(
            "radius {:.1} m, length {:.1} m",
            radius.as_f32(),
            length.as_f32()
        ),
    }
}

fn material_name(mat: &synth_awe::Material) -> String {
    use synth_awe::Material;
    // Match by absorption coefficients to identify the material
    if *mat == Material::CONCRETE {
        "Concrete"
    } else if *mat == Material::WOOD {
        "Wood"
    } else if *mat == Material::GLASS {
        "Glass"
    } else if *mat == Material::METAL {
        "Metal"
    } else if *mat == Material::FABRIC {
        "Fabric"
    } else if *mat == Material::TILE {
        "Tile"
    } else if *mat == Material::MARBLE {
        "Marble"
    } else if *mat == Material::ICE {
        "Ice"
    } else if *mat == Material::CARPET {
        "Carpet"
    } else if *mat == Material::WATER {
        "Water"
    } else if *mat == Material::VOID {
        "Void"
    } else if *mat == Material::PRISM {
        "Prism"
    } else if *mat == Material::PLASMA {
        "Plasma"
    } else if *mat == Material::MEMBRANE {
        "Membrane"
    } else if *mat == Material::NANOGEL {
        "Nanogel"
    } else {
        "Custom"
    }
    .to_string()
}

fn mapping_name(mapping: &synth_awe::NotePositionMapping) -> String {
    match mapping {
        synth_awe::NotePositionMapping::Off => "Off",
        synth_awe::NotePositionMapping::LinearX => "LinearX",
        synth_awe::NotePositionMapping::LinearY => "LinearY",
        synth_awe::NotePositionMapping::Circular => "Circular",
    }
    .to_string()
}

fn lfo_target_name(target: &synth_awe::AweLfoTarget) -> String {
    match target {
        synth_awe::AweLfoTarget::RoomLength => "RoomLength",
        synth_awe::AweLfoTarget::RoomWidth => "RoomWidth",
        synth_awe::AweLfoTarget::SourceX => "SourceX",
        synth_awe::AweLfoTarget::SourceY => "SourceY",
        synth_awe::AweLfoTarget::ListenerX => "ListenerX",
        synth_awe::AweLfoTarget::ListenerY => "ListenerY",
        synth_awe::AweLfoTarget::DryWet => "DryWet",
        synth_awe::AweLfoTarget::FreqWarp => "FreqWarp",
        synth_awe::AweLfoTarget::EarlyLate => "EarlyLate",
        synth_awe::AweLfoTarget::ModesAmount => "ModesAmount",
        synth_awe::AweLfoTarget::ResonanceBoost => "ResonanceBoost",
        synth_awe::AweLfoTarget::TailStretch => "TailStretch",
        synth_awe::AweLfoTarget::PortalAmount => "PortalAmount",
        synth_awe::AweLfoTarget::PreDelay => "PreDelay",
        synth_awe::AweLfoTarget::ModulationDepth => "ModulationDepth",
        synth_awe::AweLfoTarget::ModulationRate => "ModulationRate",
        synth_awe::AweLfoTarget::AirAbsorption => "AirAbsorption",
        synth_awe::AweLfoTarget::Width => "Width",
        synth_awe::AweLfoTarget::HighCut => "HighCut",
        synth_awe::AweLfoTarget::LowCut => "LowCut",
        synth_awe::AweLfoTarget::Temperature => "Temperature",
    }
    .to_string()
}

fn parse_lfo_target(target: &str) -> Result<synth_awe::AweLfoTarget, McpBridgeError> {
    use synth_awe::AweLfoTarget;
    match target {
        "RoomLength" => Ok(AweLfoTarget::RoomLength),
        "RoomWidth" => Ok(AweLfoTarget::RoomWidth),
        "SourceX" => Ok(AweLfoTarget::SourceX),
        "SourceY" => Ok(AweLfoTarget::SourceY),
        "ListenerX" => Ok(AweLfoTarget::ListenerX),
        "ListenerY" => Ok(AweLfoTarget::ListenerY),
        "DryWet" => Ok(AweLfoTarget::DryWet),
        "FreqWarp" => Ok(AweLfoTarget::FreqWarp),
        "EarlyLate" => Ok(AweLfoTarget::EarlyLate),
        "ModesAmount" => Ok(AweLfoTarget::ModesAmount),
        "ResonanceBoost" => Ok(AweLfoTarget::ResonanceBoost),
        "TailStretch" => Ok(AweLfoTarget::TailStretch),
        "PortalAmount" => Ok(AweLfoTarget::PortalAmount),
        "PreDelay" => Ok(AweLfoTarget::PreDelay),
        "ModulationDepth" => Ok(AweLfoTarget::ModulationDepth),
        "ModulationRate" => Ok(AweLfoTarget::ModulationRate),
        "AirAbsorption" => Ok(AweLfoTarget::AirAbsorption),
        "Width" => Ok(AweLfoTarget::Width),
        "HighCut" => Ok(AweLfoTarget::HighCut),
        "LowCut" => Ok(AweLfoTarget::LowCut),
        "Temperature" => Ok(AweLfoTarget::Temperature),
        _ => Err(McpBridgeError::InvalidLfoTarget(target.to_string())),
    }
}

fn parse_material(name: &str) -> Result<synth_awe::Material, McpBridgeError> {
    use synth_awe::Material;
    match name.to_ascii_lowercase().as_str() {
        "concrete" => Ok(Material::CONCRETE),
        "wood" => Ok(Material::WOOD),
        "glass" => Ok(Material::GLASS),
        "metal" => Ok(Material::METAL),
        "fabric" => Ok(Material::FABRIC),
        "tile" => Ok(Material::TILE),
        "marble" => Ok(Material::MARBLE),
        "ice" => Ok(Material::ICE),
        "carpet" => Ok(Material::CARPET),
        "water" => Ok(Material::WATER),
        "void" => Ok(Material::VOID),
        "prism" => Ok(Material::PRISM),
        "plasma" => Ok(Material::PLASMA),
        "membrane" => Ok(Material::MEMBRANE),
        "nanogel" => Ok(Material::NANOGEL),
        _ => Err(McpBridgeError::InvalidMaterial(name.to_string())),
    }
}

fn parse_room_shape(
    shape: &str,
    dimensions: &[f32],
) -> Result<synth_awe::RoomShape, McpBridgeError> {
    use synth_awe::{Meters, RoomShape};

    match shape.to_ascii_lowercase().as_str() {
        "box" => {
            if dimensions.len() < 3 {
                return Err(McpBridgeError::Other(format!(
                    "Box requires 3 dimensions [length, width, height], got {}. \
                     Example: [8.0, 5.0, 3.0] for an 8m x 5m x 3m room.",
                    dimensions.len()
                )));
            }
            Ok(RoomShape::Box {
                length: Meters::new(dimensions[0]),
                width: Meters::new(dimensions[1]),
                height: Meters::new(dimensions[2]),
            })
        }
        "cylinder" => {
            if dimensions.len() < 2 {
                return Err(McpBridgeError::Other(format!(
                    "Cylinder requires 2 dimensions [radius, length], got {}. \
                     Example: [1.0, 20.0] for a 1m radius, 20m long tunnel.",
                    dimensions.len()
                )));
            }
            Ok(RoomShape::Cylinder {
                radius: Meters::new(dimensions[0]),
                length: Meters::new(dimensions[1]),
            })
        }
        "lshape" | "l_shape" | "l-shape" => {
            if dimensions.len() < 5 {
                return Err(McpBridgeError::Other(format!(
                    "LShape requires 5 dimensions [length_a, width_a, length_b, width_b, height], got {}. \
                     Example: [8.0, 5.0, 6.0, 4.0, 3.0] for two connected rectangular sections.",
                    dimensions.len()
                )));
            }
            Ok(RoomShape::LShape {
                length_a: Meters::new(dimensions[0]),
                width_a: Meters::new(dimensions[1]),
                length_b: Meters::new(dimensions[2]),
                width_b: Meters::new(dimensions[3]),
                height: Meters::new(dimensions[4]),
            })
        }
        "sphere" => {
            if dimensions.is_empty() {
                return Err(McpBridgeError::Other(
                    "Sphere requires 1 dimension [radius]. \
                     Example: [5.0] for a 5m radius sphere."
                        .to_string(),
                ));
            }
            Ok(RoomShape::Sphere {
                radius: Meters::new(dimensions[0]),
            })
        }
        "dome" => {
            if dimensions.is_empty() {
                return Err(McpBridgeError::Other(
                    "Dome requires 1 dimension [radius]. \
                     Example: [6.0] for a 6m radius dome (height = radius)."
                        .to_string(),
                ));
            }
            Ok(RoomShape::Dome {
                radius: Meters::new(dimensions[0]),
            })
        }
        "tube" => {
            if dimensions.len() < 2 {
                return Err(McpBridgeError::Other(format!(
                    "Tube requires 2 dimensions [radius, length], got {}. \
                     Example: [1.5, 30.0] for a 1.5m radius, 30m long open tube.",
                    dimensions.len()
                )));
            }
            Ok(RoomShape::Tube {
                radius: Meters::new(dimensions[0]),
                length: Meters::new(dimensions[1]),
            })
        }
        _ => Err(McpBridgeError::InvalidRoomShape(shape.to_string())),
    }
}

/// Apply a single AWE param to the shared state snapshot.
fn apply_awe_param_to_state(state: &mut synth_awe::AweState, param: &synth_awe::AweParam) {
    use synth_awe::AweParam;
    match param {
        AweParam::RoomShape(v) => state.room = *v,
        AweParam::Material(v) => state.material = *v,
        AweParam::SourcePos(v) => state.snapshot.source_pos = *v,
        AweParam::ListenerPos(v) => state.snapshot.listener_pos = *v,
        AweParam::DryWet(v) => state.snapshot.dry_wet = *v,
        AweParam::EarlyLateBalance(v) => state.snapshot.early_late_balance = *v,
        AweParam::ModesAmount(v) => state.snapshot.modes_amount = *v,
        AweParam::FreqWarp(v) => state.snapshot.freq_warp = *v,
        AweParam::ResonanceBoost(v) => state.snapshot.resonance_boost = *v,
        AweParam::TailStretch(v) => state.snapshot.tail_stretch = *v,
        AweParam::PortalAmount(v) => state.snapshot.portal_amount = *v,
        AweParam::PreDelay(v) => state.snapshot.pre_delay = *v,
        AweParam::Enabled(v) => state.enabled = *v,
        AweParam::SpatialEnabled(v) => {
            state.spatial_enabled = *v;
            state.snapshot.spatial_enabled = *v;
        }
        AweParam::NoteMapping(v) => {
            state.note_mapping = *v;
            state.snapshot.note_mapping = *v;
        }
        AweParam::Lfo1Rate(v) => state.snapshot.lfo1.rate = *v,
        AweParam::Lfo1Amount(v) => state.snapshot.lfo1.amount = *v,
        AweParam::Lfo1Target(v) => state.snapshot.lfo1.target = *v,
        AweParam::Lfo2Rate(v) => state.snapshot.lfo2.rate = *v,
        AweParam::Lfo2Amount(v) => state.snapshot.lfo2.amount = *v,
        AweParam::Lfo2Target(v) => state.snapshot.lfo2.target = *v,
        AweParam::Lfo3Rate(v) => state.snapshot.lfo3.rate = *v,
        AweParam::Lfo3Amount(v) => state.snapshot.lfo3.amount = *v,
        AweParam::Lfo3Target(v) => state.snapshot.lfo3.target = *v,
        AweParam::Lfo4Rate(v) => state.snapshot.lfo4.rate = *v,
        AweParam::Lfo4Amount(v) => state.snapshot.lfo4.amount = *v,
        AweParam::Lfo4Target(v) => state.snapshot.lfo4.target = *v,
        AweParam::ModulationDepth(v) => state.snapshot.modulation_depth = *v,
        AweParam::ModulationRate(v) => state.snapshot.modulation_rate = *v,
        AweParam::AirAbsorption(v) => state.snapshot.air_absorption = *v,
        AweParam::Width(v) => state.snapshot.width = *v,
        AweParam::HighCut(v) => state.snapshot.high_cut = *v,
        AweParam::LowCut(v) => state.snapshot.low_cut = *v,
        AweParam::Temperature(v) => state.snapshot.temperature = *v,
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

/// Try to insert a note from `BridgeNoteData` into a pattern.
/// Returns the assigned note ID as u64, or an error string if the pitch is invalid.
fn try_insert_note_into_pattern(
    pattern: &mut synth_sequencer::Pattern,
    n: &BridgeNoteData,
) -> Result<u64, String> {
    let pitch = synth_sequencer::Pitch::new(n.pitch)
        .ok_or_else(|| format!("invalid pitch: {} (must be 0..=127)", n.pitch))?;
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

    Ok(pattern.insert_note(note).0)
}

/// Insert a note from `BridgeNoteData` into a pattern. Returns the assigned note ID as u64.
/// Falls back to middle C for invalid pitches (used in bulk import paths where per-note
/// errors are not reported).
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

/// Render a note offline and compute analysis metrics from the f32 buffer.
///
/// Shared between the MCP bridge `analyze_note` method and any other caller
/// that wants quantitative metrics rather than an opaque WAV blob.
///
/// `expected_note` (when `Some`) anchors `expected_fundamental_hz` to that
/// MIDI note and narrows the fundamental search to ±tritone around it. This
/// keeps the pitch metric meaningful for patches where the loudest spectral
/// peak is not the fundamental (sub-bass with a dominant sub-osc, wave-folded
/// patches with redistributed harmonics, etc.).
#[allow(clippy::too_many_arguments)]
fn analyze_rendered_note(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    instrument_id: u64,
    note: u8,
    velocity: u8,
    duration_ms: u32,
    tail_ms: u32,
    expected_note: Option<u8>,
) -> Result<synth_mcp::types::AnalyzeNoteResult, McpBridgeError> {
    let rendered = crate::audio::preview::render_note_to_buffer(
        session,
        sample_library,
        InstrumentId::new(instrument_id),
        MidiNote::new(note),
        Velocity::from_midi(velocity),
        duration_ms,
        tail_ms,
    )?;

    Ok(analyze_rendered_buffer(
        &rendered,
        note,
        velocity,
        duration_ms,
        expected_note,
    ))
}

/// Default note duration for sweep tools. Long enough for the envelope to
/// reach sustain on typical patches; short enough that 60-note sweeps don't
/// take minutes.
const SWEEP_DEFAULT_DURATION_MS: u32 = 400;
/// Default release tail for sweep tools.
const SWEEP_DEFAULT_TAIL_MS: u32 = 200;
const SWEEP_DEFAULT_STEP_SEMITONES: u8 = 12;
const SWEEP_DEFAULT_VELOCITY: u8 = 100;
const SWEEP_DEFAULT_VELOCITY_LOW: u8 = 1;
const SWEEP_DEFAULT_VELOCITY_HIGH: u8 = 127;
const SWEEP_DEFAULT_VELOCITY_STEP: u8 = 16;
const SWEEP_NYQUIST_HZ: f32 = crate::audio::preview::PREVIEW_SAMPLE_RATE as f32 / 2.0;

/// Walk `lo..=hi` in `step` increments, always including `hi` as the final
/// value. Calls `on_step` per value; pushes a `"{label} {val}: render failed: {e}"`
/// warning when `on_step` errors and continues the sweep.
fn sweep_range<T>(
    lo: u8,
    hi: u8,
    step: u8,
    label: &str,
    warnings: &mut Vec<String>,
    mut on_step: impl FnMut(u8) -> Result<T, McpBridgeError>,
) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    let mut val = lo;
    loop {
        match on_step(val) {
            Ok(t) => out.push(t),
            Err(e) => warnings.push(format!("{label} {val}: render failed: {e}")),
        }
        if val == hi {
            break;
        }
        val = val.saturating_add(step).min(hi);
    }
    out
}

/// Sweep an instrument across a MIDI note range. One offline render per step,
/// reuses the `analyze_note` path; cross-step issues are derived in
/// `analysis::patch_sweep`.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn analyze_instrument_range_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    instrument_id: u64,
    low_note: u8,
    high_note: u8,
    step_semitones: Option<u8>,
    velocity: Option<u8>,
    duration_ms: Option<u32>,
    tail_ms: Option<u32>,
) -> Result<synth_mcp::types::AnalyzeInstrumentRangeResult, McpBridgeError> {
    use crate::analysis::patch_sweep::{range_issues_from_steps, range_step_from_analysis};

    if low_note > high_note {
        return Err(McpBridgeError::Other(format!(
            "low_note ({low_note}) must be <= high_note ({high_note})"
        )));
    }
    let step = step_semitones
        .unwrap_or(SWEEP_DEFAULT_STEP_SEMITONES)
        .max(1);
    let velocity = velocity.unwrap_or(SWEEP_DEFAULT_VELOCITY);
    let duration_ms = duration_ms.unwrap_or(SWEEP_DEFAULT_DURATION_MS);
    let tail_ms = tail_ms.unwrap_or(SWEEP_DEFAULT_TAIL_MS);

    let mut warnings: Vec<String> = Vec::new();
    let steps_out = sweep_range(low_note, high_note, step, "note", &mut warnings, |note| {
        let result = analyze_rendered_note(
            session,
            sample_library,
            instrument_id,
            note,
            velocity,
            duration_ms,
            tail_ms,
            Some(note),
        )?;
        Ok(range_step_from_analysis(note, &result, SWEEP_NYQUIST_HZ))
    });

    let issues = range_issues_from_steps(&steps_out);
    Ok(synth_mcp::types::AnalyzeInstrumentRangeResult {
        instrument_id,
        velocity,
        low_note,
        high_note,
        step_semitones: step,
        duration_ms,
        tail_ms,
        steps: steps_out,
        issues,
        warnings,
    })
}

/// Hold one note and sweep velocity. Same render path as
/// `analyze_instrument_range`, but the note is fixed and velocity walks the
/// range.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn analyze_velocity_response_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    instrument_id: u64,
    note: u8,
    velocity_low: Option<u8>,
    velocity_high: Option<u8>,
    velocity_step: Option<u8>,
    duration_ms: Option<u32>,
    tail_ms: Option<u32>,
) -> Result<synth_mcp::types::AnalyzeVelocityResponseResult, McpBridgeError> {
    use crate::analysis::patch_sweep::{velocity_issues_from_steps, velocity_step_from_analysis};

    let velocity_low = velocity_low.unwrap_or(SWEEP_DEFAULT_VELOCITY_LOW).max(1);
    let velocity_high = velocity_high
        .unwrap_or(SWEEP_DEFAULT_VELOCITY_HIGH)
        .min(127);
    if velocity_low > velocity_high {
        return Err(McpBridgeError::Other(format!(
            "velocity_low ({velocity_low}) must be <= velocity_high ({velocity_high})"
        )));
    }
    let velocity_step = velocity_step.unwrap_or(SWEEP_DEFAULT_VELOCITY_STEP).max(1);
    let duration_ms = duration_ms.unwrap_or(SWEEP_DEFAULT_DURATION_MS);
    let tail_ms = tail_ms.unwrap_or(SWEEP_DEFAULT_TAIL_MS);

    let mut warnings: Vec<String> = Vec::new();
    let steps_out = sweep_range(
        velocity_low,
        velocity_high,
        velocity_step,
        "velocity",
        &mut warnings,
        |velocity| {
            let result = analyze_rendered_note(
                session,
                sample_library,
                instrument_id,
                note,
                velocity,
                duration_ms,
                tail_ms,
                Some(note),
            )?;
            Ok(velocity_step_from_analysis(velocity, &result))
        },
    );

    let issues = velocity_issues_from_steps(&steps_out);
    Ok(synth_mcp::types::AnalyzeVelocityResponseResult {
        instrument_id,
        note,
        velocity_low,
        velocity_high,
        velocity_step,
        duration_ms,
        tail_ms,
        steps: steps_out,
        issues,
        warnings,
    })
}

/// Default chord-detection window for pattern-scope analysis: one quarter
/// note at 960 PPQN. Patterns are short enough that fine resolution keeps
/// the output compact.
const DEFAULT_PATTERN_GROUPING_TICKS: u32 = 960;

/// Default chord-detection window for arrangement-scope analysis: one bar
/// (assumed 4/4). Arrangements span many bars and a per-quarter resolution
/// blows past the MCP response-size limit; per-bar resolution keeps the
/// chord-event list readable. Callers can override with a smaller value.
const DEFAULT_ARRANGEMENT_GROUPING_TICKS: u32 = 3840;

/// End tick for an open-ended note (no `duration`): one grouping window
/// past `start` so the note contributes weight to exactly one chord event.
fn synthetic_note_end(start: u32, grouping_ticks: u32) -> u32 {
    start.saturating_add(grouping_ticks)
}

/// Convert an absolute tick to 1-indexed (bar, beat) under the given time
/// signature. `Tick::to_bar_beat_tick` returns 0-indexed values; we shift
/// to 1-indexed for human readability ("Bar 1 beat 1" = song start).
fn tick_to_bar_beat_1based(tick: u64, time_sig: synth_sequencer::TimeSignature) -> (u32, u32) {
    let (bar, beat, _) = synth_sequencer::Tick(tick).to_bar_beat_tick(time_sig);
    (bar + 1, beat + 1)
}

/// Merge consecutive `HarmonyChordEvent`s that share a chord symbol (or are
/// both unidentified) into single spans. Keeps the chord-event list compact
/// when a chord is held for several grouping windows.
fn merge_consecutive_chord_events(events: Vec<HarmonyChordEvent>) -> Vec<HarmonyChordEvent> {
    let mut out: Vec<HarmonyChordEvent> = Vec::with_capacity(events.len());
    for e in events {
        if let Some(last) = out.last_mut()
            && last.symbol == e.symbol
            && last.in_key == e.in_key
        {
            last.end_tick = e.end_tick;
            for m in e.midi_notes {
                if !last.midi_notes.contains(&m) {
                    last.midi_notes.push(m);
                }
            }
            last.midi_notes.sort_unstable();
            continue;
        }
        out.push(e);
    }
    out
}

/// Implementation of the `analyze_harmony` bridge method.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn analyze_song_harmony(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<u32>,
    arrangement_start_tick: Option<u64>,
    arrangement_end_tick: Option<u64>,
    grouping_ticks: Option<u32>,
    exclude_drums: Option<bool>,
    exclude_track_ids: Option<Vec<u16>>,
) -> Result<AnalyzeHarmonyResult, McpBridgeError> {
    use synth_sequencer::{PatternId, SeqInstrumentId, TrackId};

    let song = shared.song.read();
    let default_grouping = if pattern_id.is_some() {
        DEFAULT_PATTERN_GROUPING_TICKS
    } else {
        DEFAULT_ARRANGEMENT_GROUPING_TICKS
    };
    let grouping = grouping_ticks
        .filter(|g| *g > 0)
        .unwrap_or(default_grouping);
    let exclude_drums = exclude_drums.unwrap_or(true);
    let explicit_excluded: std::collections::HashSet<TrackId> = exclude_track_ids
        .as_deref()
        .map(|ids| ids.iter().copied().map(TrackId).collect())
        .unwrap_or_default();
    let mut warnings: Vec<String> = Vec::new();

    // Lock in the time signature once per request so chord-event bar/beat
    // formatting is consistent across the whole scope. Mid-arrangement time
    // signature changes will report bar/beat under the time signature at the
    // scope start; that's accurate enough for the typical case where TS
    // changes are rare.
    let default_ts = song.default_time_signature;
    let scope_time_signature = match (pattern_id, arrangement_start_tick) {
        (Some(_), _) => default_ts,
        (None, Some(t)) => song.time_signature_at(synth_sequencer::Tick(t)),
        (None, None) => default_ts,
    };

    let (scope, notes, range_start, range_end) = match pattern_id {
        Some(pid) => {
            if !explicit_excluded.is_empty() {
                warnings.push(
                    "exclude_track_ids is ignored in pattern scope — a pattern is not tied to a specific track".to_string(),
                );
            }
            let pid_typed = PatternId(pid);
            let Some(pattern) = song.pattern(pid_typed) else {
                return Err(McpBridgeError::Other(format!("Pattern {pid} not found")));
            };
            let length_ticks = pattern.length.0;
            let mut notes = Vec::with_capacity(pattern.notes().len());
            for n in pattern.notes() {
                let start_pt = n.start.0;
                let end_pt = match n.duration {
                    Some(d) => start_pt.saturating_add(d.0),
                    None => synthetic_note_end(start_pt, grouping),
                };
                notes.push(crate::harmony::AnalysisNote {
                    pitch: n.pitch,
                    start_tick: u64::from(start_pt),
                    end_tick: u64::from(end_pt),
                });
            }
            if notes.is_empty() {
                warnings.push(format!("Pattern {pid} contains no notes"));
            }
            (
                HarmonyScope::Pattern { pattern_id: pid },
                notes,
                0u64,
                u64::from(length_ticks),
            )
        }
        None => {
            let (start, end) =
                resolve_arrangement_range(&song, arrangement_start_tick, arrangement_end_tick)?;

            // Resolve which tracks to skip. A track is excluded when either:
            //   1. It appears in the explicit `exclude_track_ids` list, or
            //   2. `exclude_drums` is true and `infer_all_profiles` classifies
            //      its assigned instrument as Drums with confidence >= 0.6.
            //      Manual `set_instrument_category` still wins (it produces
            //      role Drums with confidence 1.0 via manual-override), but
            //      the inference also catches percussion that was never
            //      manually tagged — closing the §8.2 silent-no-op.
            // Tracks with no instrument assignment are never auto-excluded.
            let drum_profiles: std::collections::HashMap<
                SeqInstrumentId,
                crate::analysis::InstrumentProfile,
            > = if exclude_drums {
                crate::analysis::infer_all_profiles(&song, session.state())
                    .into_iter()
                    .filter(|p| {
                        p.role.role == crate::analysis::Role::Drums && p.role.confidence >= 0.6
                    })
                    .map(|p| (SeqInstrumentId(p.instrument_id), p))
                    .collect()
            } else {
                std::collections::HashMap::new()
            };
            let auto_excluded_tracks: std::collections::HashSet<TrackId> = song
                .tracks()
                .filter_map(|t| {
                    let seq = t.instrument?;
                    drum_profiles.contains_key(&seq).then_some(t.id)
                })
                .collect();
            let excluded_tracks: std::collections::HashSet<TrackId> = auto_excluded_tracks
                .iter()
                .chain(explicit_excluded.iter())
                .copied()
                .collect();
            if !excluded_tracks.is_empty() {
                let descriptions: Vec<String> = song
                    .tracks()
                    .filter(|t| excluded_tracks.contains(&t.id))
                    .map(|t| {
                        let base = format!("{}({})", t.name, t.id.0);
                        // Signal trail only for drum-auto-excludes — explicit
                        // excludes have no inference behind them.
                        if let Some(seq) = t.instrument
                            && let Some(profile) = drum_profiles.get(&seq)
                        {
                            let sigs = profile
                                .role
                                .signals
                                .iter()
                                .map(|s| format!("{}:{}", s.axis, s.detail))
                                .collect::<Vec<_>>()
                                .join(", ");
                            return format!(
                                "{base} [drums conf={:.2}; {sigs}]",
                                profile.role.confidence
                            );
                        }
                        base
                    })
                    .collect();
                warnings.push(format!(
                    "Excluded {} track(s) from harmony analysis: {}",
                    excluded_tracks.len(),
                    descriptions.join(", ")
                ));
            }

            let mut notes: Vec<crate::harmony::AnalysisNote> = Vec::new();
            for placement in
                song.placements_in_range(synth_sequencer::Tick(start), synth_sequencer::Tick(end))
            {
                if excluded_tracks.contains(&placement.track_id) {
                    continue;
                }
                let Some(pattern) = song.pattern(placement.pattern_id) else {
                    continue;
                };
                let placement_start = placement.start.0;
                for n in pattern.notes() {
                    let n_start = n.start.0;
                    let n_end_pt = match n.duration {
                        Some(d) => n_start.saturating_add(d.0),
                        None => synthetic_note_end(n_start, grouping),
                    };
                    let abs_start = placement_start.saturating_add(u64::from(n_start));
                    let abs_end = placement_start.saturating_add(u64::from(n_end_pt));
                    if abs_end <= start || abs_start >= end {
                        continue;
                    }
                    let transposed = n.pitch.transpose(placement.transpose);
                    let Some(pitch) = transposed else {
                        warnings.push(format!(
                            "Note at tick {abs_start} dropped: transpose out of MIDI range"
                        ));
                        continue;
                    };
                    notes.push(crate::harmony::AnalysisNote {
                        pitch,
                        start_tick: abs_start,
                        end_tick: abs_end,
                    });
                }
            }
            if notes.is_empty() {
                warnings.push("No notes found in arrangement range".to_string());
            }
            (
                HarmonyScope::Arrangement {
                    start_tick: start,
                    end_tick: end,
                },
                notes,
                start,
                end,
            )
        }
    };

    drop(song);

    let opts = crate::harmony::AnalysisOptions {
        grouping_ticks: u64::from(grouping),
        range_start_tick: range_start,
        range_end_tick: range_end,
    };
    let analysis = crate::harmony::analyze(&notes, &opts);

    // Convert the harmony module's output into the serializable MCP types.
    // Track identified / distinct counts on the raw per-window events so the
    // stats reflect the analyzer's resolution, not the post-merge view.
    let mut distinct = std::collections::HashSet::new();
    let mut identified = 0u32;
    let raw_chords: Vec<HarmonyChordEvent> = analysis
        .events
        .iter()
        .map(|e| {
            let (symbol, root, quality) = match &e.chord {
                Some(c) => {
                    distinct.insert(c.symbol.clone());
                    identified += 1;
                    (
                        Some(c.symbol.clone()),
                        Some(c.root),
                        Some(c.quality.to_string()),
                    )
                }
                None => (None, None, None),
            };
            let (start_bar, start_beat) =
                tick_to_bar_beat_1based(e.start_tick, scope_time_signature);
            HarmonyChordEvent {
                start_bar,
                start_beat,
                start_tick: e.start_tick,
                end_tick: e.end_tick,
                midi_notes: e.midi_notes.clone(),
                symbol,
                root,
                quality,
                in_key: e.in_key,
            }
        })
        .collect();
    let raw_event_count = raw_chords.len() as u32;
    let chords = merge_consecutive_chord_events(raw_chords);

    let to_key_estimate = |k: &crate::harmony::KeyEstimate| HarmonyKeyEstimate {
        tonic: k.tonic,
        tonic_name: synth_sequencer::NoteName::from_midi(k.tonic).to_string(),
        mode: k.mode.to_string(),
        label: k.label(),
        correlation: k.correlation,
    };

    let avg_polyphony = if chords.is_empty() {
        0.0
    } else {
        chords
            .iter()
            .map(|c| c.midi_notes.len() as f32)
            .sum::<f32>()
            / chords.len() as f32
    };
    let (lo, hi) = analysis.pitch_range.unwrap_or((0, 0));
    let stats = HarmonyStats {
        total_notes: analysis.total_notes,
        chord_event_count: raw_event_count,
        distinct_chord_count: distinct.len() as u32,
        identified_chord_count: identified,
        pitch_range_low: lo,
        pitch_range_high: hi,
        avg_polyphony,
        grouping_ticks: grouping,
    };

    Ok(AnalyzeHarmonyResult {
        scope,
        chords,
        inferred_key: analysis.inferred_key.as_ref().map(to_key_estimate),
        key_candidates: analysis
            .key_candidates
            .iter()
            .map(to_key_estimate)
            .collect(),
        pitch_class_histogram: analysis.histogram,
        in_key_ratio: analysis.in_key_ratio,
        out_of_scale_pitch_classes: analysis.out_of_scale_pcs,
        harmonic_stability_score: analysis.harmonic_stability_score,
        stats,
        warnings,
    })
}

fn analyze_pattern_impl(
    shared: &McpSharedState,
    pattern_id: u32,
) -> Result<AnalyzePatternResult, McpBridgeError> {
    use synth_mcp::types::{
        AnalyzePatternResult, PatternDensity, PatternPitch, PatternRepetition, PatternRhythm,
        PatternVelocity,
    };
    use synth_sequencer::PatternId;

    let song = shared.song.read();
    let pid = PatternId(pattern_id);
    let Some(pattern) = song.pattern(pid) else {
        return Err(McpBridgeError::Other(format!(
            "Pattern {pattern_id} not found"
        )));
    };
    let length_ticks = pattern.length.0;
    let pattern_name = pattern.name.clone();
    let notes: Vec<synth_sequencer::Note> = pattern.notes().to_vec();
    let time_sig = song.default_time_signature;
    drop(song);

    let analysis = crate::analysis::pattern_analysis::analyze(&notes, length_ticks, time_sig);

    Ok(AnalyzePatternResult {
        pattern_id,
        pattern_name,
        length_ticks: analysis.length_ticks,
        length_bars: analysis.length_bars,
        time_signature_numerator: time_sig.numerator,
        time_signature_denominator: time_sig.denominator,
        note_count: analysis.note_count,
        density: PatternDensity {
            notes_per_bar: analysis.density.notes_per_bar,
            notes_per_beat: analysis.density.notes_per_beat,
            active_ratio: analysis.density.active_ratio,
        },
        pitch: PatternPitch {
            low: analysis.pitch.low,
            high: analysis.pitch.high,
            range_semitones: analysis.pitch.range_semitones,
            mean: analysis.pitch.mean,
            distinct_count: analysis.pitch.distinct_count,
            class_histogram: analysis.pitch.class_histogram,
        },
        velocity: PatternVelocity {
            min: analysis.velocity.min,
            max: analysis.velocity.max,
            mean: analysis.velocity.mean,
            std_dev: analysis.velocity.std_dev,
            range: analysis.velocity.range,
        },
        rhythm: PatternRhythm {
            max_polyphony: analysis.rhythm.max_polyphony,
            mean_polyphony: analysis.rhythm.mean_polyphony,
            is_monophonic: analysis.rhythm.is_monophonic,
            distinct_onset_count: analysis.rhythm.distinct_onset_count,
            distinct_duration_count: analysis.rhythm.distinct_duration_count,
            mean_ioi_ticks: analysis.rhythm.mean_ioi_ticks,
            ioi_std_ticks: analysis.rhythm.ioi_std_ticks,
            regularity_score: analysis.rhythm.regularity_score,
        },
        repetition: PatternRepetition {
            distinct_bars: analysis.repetition.distinct_bars,
            total_bars: analysis.repetition.total_bars,
            bar_repetition_score: analysis.repetition.bar_repetition_score,
        },
        warnings: analysis.warnings,
    })
}

/// Resolve the arrangement range used by `analyze_drum_groove` /
/// `analyze_bass_drum_lock` / `analyze_harmonic_function` when the caller
/// passes `start`/`end` as `None`.
fn resolve_arrangement_range(
    song: &synth_sequencer::Song,
    start: Option<u64>,
    end: Option<u64>,
) -> Result<(u64, u64), McpBridgeError> {
    let song_end = song.calculate_length().0;
    let start = start.unwrap_or(0);
    let end = end.unwrap_or(song_end);
    if end <= start {
        return Err(McpBridgeError::Other(format!(
            "Arrangement range invalid: end ({end}) must be greater than start ({start})"
        )));
    }
    Ok((start, end))
}

fn analyze_drum_groove_impl(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<u32>,
    arrangement_start_tick: Option<u64>,
    arrangement_end_tick: Option<u64>,
) -> Result<synth_mcp::types::AnalyzeDrumGrooveResult, McpBridgeError> {
    use synth_mcp::types::{
        AnalyzeDrumGrooveResult, DrumBackbeat, DrumComposition, DrumFills, DrumGhostNotes, DrumHat,
        DrumRepetition, DrumTrackInfo, HarmonyScope,
    };
    use synth_sequencer::{PatternId, SeqInstrumentId};

    let song = shared.song.read();
    let mut warnings: Vec<String> = Vec::new();

    let (scope, time_sig, length_ticks, notes, drum_tracks, start_tick, end_tick) = match pattern_id
    {
        Some(pid) => {
            let pid_typed = PatternId(pid);
            let Some(pattern) = song.pattern(pid_typed) else {
                return Err(McpBridgeError::Other(format!("Pattern {pid} not found")));
            };
            let length_ticks = pattern.length.0;
            let notes: Vec<crate::analysis::DrumNote> = pattern
                .notes()
                .iter()
                .map(|n| crate::analysis::DrumNote::from_note(n, 0))
                .collect();
            let ts = song.default_time_signature;
            (
                HarmonyScope::Pattern { pattern_id: pid },
                ts,
                length_ticks,
                notes,
                Vec::<DrumTrackInfo>::new(),
                0u64,
                u64::from(length_ticks),
            )
        }
        None => {
            let (start, end) =
                resolve_arrangement_range(&song, arrangement_start_tick, arrangement_end_tick)?;

            // Find drum-track candidates the same way `analyze_harmony` does.
            let drum_profiles: std::collections::HashMap<
                SeqInstrumentId,
                crate::analysis::InstrumentProfile,
            > = crate::analysis::infer_all_profiles(&song, session.state())
                .into_iter()
                .filter(|p| p.role.role == crate::analysis::Role::Drums && p.role.confidence >= 0.6)
                .map(|p| (SeqInstrumentId(p.instrument_id), p))
                .collect();

            if drum_profiles.is_empty() {
                warnings.push(
                    "No drum tracks identified by infer_all_profiles (confidence >= 0.6)"
                        .to_string(),
                );
            }

            let mut drum_track_infos: Vec<DrumTrackInfo> = song
                .tracks()
                .filter_map(|t| {
                    let seq = t.instrument?;
                    let profile = drum_profiles.get(&seq)?;
                    Some(DrumTrackInfo {
                        track_id: t.id.0,
                        track_name: t.name.clone(),
                        instrument_id: seq.0,
                        instrument_name: profile.instrument_name.clone(),
                        drum_confidence: profile.role.confidence,
                    })
                })
                .collect();
            drum_track_infos.sort_by_key(|d| d.track_id);

            let drum_track_ids: std::collections::HashSet<synth_sequencer::TrackId> =
                drum_track_infos
                    .iter()
                    .map(|d| synth_sequencer::TrackId(d.track_id))
                    .collect();

            let mut notes: Vec<crate::analysis::DrumNote> = Vec::new();
            for placement in
                song.placements_in_range(synth_sequencer::Tick(start), synth_sequencer::Tick(end))
            {
                if !drum_track_ids.contains(&placement.track_id) {
                    continue;
                }
                let Some(pattern) = song.pattern(placement.pattern_id) else {
                    continue;
                };
                let placement_start = placement.start.0;
                for n in pattern.notes() {
                    let abs_start = placement_start.saturating_add(u64::from(n.start.0));
                    if abs_start < start || abs_start >= end {
                        continue;
                    }
                    // Drum analysis works in range-relative tick space so
                    // `length_ticks = end - start` and per-bar math lines
                    // up with the analyzed window.
                    let rel = (abs_start - start) as u32;
                    notes.push(crate::analysis::DrumNote {
                        tick: rel,
                        midi: n.pitch.as_midi(),
                        velocity: n.velocity.as_f32(),
                    });
                }
            }

            let length_ticks: u32 = (end - start).try_into().unwrap_or(u32::MAX);
            let ts = song.time_signature_at(synth_sequencer::Tick(start));
            (
                HarmonyScope::Arrangement {
                    start_tick: start,
                    end_tick: end,
                },
                ts,
                length_ticks,
                notes,
                drum_track_infos,
                start,
                end,
            )
        }
    };

    drop(song);

    let mut analysis = crate::analysis::drum_groove::analyze(&notes, length_ticks, time_sig);
    warnings.append(&mut analysis.warnings);

    let (start_bar, start_beat) = tick_to_bar_beat_1based(start_tick, time_sig);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(end_tick, time_sig);

    Ok(AnalyzeDrumGrooveResult {
        scope,
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        length_ticks: analysis.length_ticks,
        length_bars: analysis.length_bars,
        time_signature_numerator: time_sig.numerator,
        time_signature_denominator: time_sig.denominator,
        total_drum_notes: analysis.total_drum_notes,
        drum_tracks,
        composition: DrumComposition {
            kick: analysis.composition.kick,
            snare: analysis.composition.snare,
            hat_closed: analysis.composition.hat_closed,
            hat_open: analysis.composition.hat_open,
            tom: analysis.composition.tom,
            cymbal: analysis.composition.cymbal,
            clap: analysis.composition.clap,
            other: analysis.composition.other,
        },
        backbeat: DrumBackbeat {
            strength: analysis.backbeat.strength,
            expected_backbeats: analysis.backbeat.expected_backbeats,
            matched_backbeats: analysis.backbeat.matched_backbeats,
            off_backbeat_snares: analysis.backbeat.off_backbeat_snares,
        },
        hat: DrumHat {
            subdivision: analysis.hat.subdivision,
            hat_density_per_beat: analysis.hat.hat_density_per_beat,
            hat_count: analysis.hat.hat_count,
        },
        ghost_notes: DrumGhostNotes {
            count: analysis.ghost_notes.count,
            velocity_threshold: analysis.ghost_notes.velocity_threshold,
        },
        fills: DrumFills {
            fill_bar_count: analysis.fills.fill_bar_count,
            density_threshold: analysis.fills.density_threshold,
            mean_density_per_bar: analysis.fills.mean_density_per_bar,
        },
        repetition: DrumRepetition {
            distinct_bars: analysis.repetition.distinct_bars,
            total_bars: analysis.repetition.total_bars,
            bar_repetition_score: analysis.repetition.bar_repetition_score,
        },
        warnings,
    })
}

fn analyze_bass_drum_lock_impl(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<u32>,
    arrangement_start_tick: Option<u64>,
    arrangement_end_tick: Option<u64>,
    onset_tolerance_ticks: Option<u32>,
) -> Result<synth_mcp::types::AnalyzeBassDrumLockResult, McpBridgeError> {
    use synth_mcp::types::{
        AnalyzeBassDrumLockResult, BassDrumAlignment, BassPitchStability, BassTrackInfo,
        DrumTrackInfo, HarmonyScope,
    };
    use synth_sequencer::{PatternId, SeqInstrumentId};

    let song = shared.song.read();
    let mut warnings: Vec<String> = Vec::new();
    let tolerance = onset_tolerance_ticks
        .unwrap_or(crate::analysis::bass_drum_lock::DEFAULT_ONSET_TOLERANCE_TICKS);

    let (
        scope,
        time_sig,
        length_ticks,
        kicks,
        bass,
        drum_tracks,
        bass_tracks,
        start_tick,
        end_tick,
    ) = match pattern_id {
        Some(pid) => {
            let pid_typed = PatternId(pid);
            let Some(pattern) = song.pattern(pid_typed) else {
                return Err(McpBridgeError::Other(format!("Pattern {pid} not found")));
            };
            let length_ticks = pattern.length.0;
            let mut kicks: Vec<crate::analysis::KickOnset> = Vec::new();
            let mut bass: Vec<crate::analysis::BassOnset> = Vec::new();
            for n in pattern.notes() {
                let midi = n.pitch.as_midi();
                if matches!(
                    crate::analysis::DrumComponent::from_midi(midi),
                    crate::analysis::DrumComponent::Kick
                ) {
                    kicks.push(crate::analysis::KickOnset { tick: n.start.0 });
                } else {
                    bass.push(crate::analysis::BassOnset {
                        tick: n.start.0,
                        midi,
                    });
                }
            }
            (
                HarmonyScope::Pattern { pattern_id: pid },
                song.default_time_signature,
                length_ticks,
                kicks,
                bass,
                Vec::<DrumTrackInfo>::new(),
                Vec::<BassTrackInfo>::new(),
                0u64,
                u64::from(length_ticks),
            )
        }
        None => {
            let (start, end) =
                resolve_arrangement_range(&song, arrangement_start_tick, arrangement_end_tick)?;

            let profiles = crate::analysis::infer_all_profiles(&song, session.state());
            let drum_profiles: std::collections::HashMap<
                SeqInstrumentId,
                crate::analysis::InstrumentProfile,
            > = profiles
                .iter()
                .filter(|p| p.role.role == crate::analysis::Role::Drums && p.role.confidence >= 0.6)
                .cloned()
                .map(|p| (SeqInstrumentId(p.instrument_id), p))
                .collect();
            let bass_profiles: std::collections::HashMap<
                SeqInstrumentId,
                crate::analysis::InstrumentProfile,
            > = profiles
                .into_iter()
                .filter(|p| p.role.role == crate::analysis::Role::Bass && p.role.confidence >= 0.6)
                .map(|p| (SeqInstrumentId(p.instrument_id), p))
                .collect();

            if drum_profiles.is_empty() {
                warnings.push(
                    "No drum tracks identified by infer_all_profiles — kick onset count will be 0"
                        .to_string(),
                );
            }
            if bass_profiles.is_empty() {
                warnings.push(
                    "No bass tracks identified by infer_all_profiles — bass onset count will be 0"
                        .to_string(),
                );
            }

            let mut drum_track_infos: Vec<DrumTrackInfo> = song
                .tracks()
                .filter_map(|t| {
                    let seq = t.instrument?;
                    let profile = drum_profiles.get(&seq)?;
                    Some(DrumTrackInfo {
                        track_id: t.id.0,
                        track_name: t.name.clone(),
                        instrument_id: seq.0,
                        instrument_name: profile.instrument_name.clone(),
                        drum_confidence: profile.role.confidence,
                    })
                })
                .collect();
            drum_track_infos.sort_by_key(|d| d.track_id);
            let mut bass_track_infos: Vec<BassTrackInfo> = song
                .tracks()
                .filter_map(|t| {
                    let seq = t.instrument?;
                    let profile = bass_profiles.get(&seq)?;
                    Some(BassTrackInfo {
                        track_id: t.id.0,
                        track_name: t.name.clone(),
                        instrument_id: seq.0,
                        instrument_name: profile.instrument_name.clone(),
                        bass_confidence: profile.role.confidence,
                    })
                })
                .collect();
            bass_track_infos.sort_by_key(|b| b.track_id);

            let drum_track_ids: std::collections::HashSet<synth_sequencer::TrackId> =
                drum_track_infos
                    .iter()
                    .map(|d| synth_sequencer::TrackId(d.track_id))
                    .collect();
            let bass_track_ids: std::collections::HashSet<synth_sequencer::TrackId> =
                bass_track_infos
                    .iter()
                    .map(|b| synth_sequencer::TrackId(b.track_id))
                    .collect();

            let mut kicks: Vec<crate::analysis::KickOnset> = Vec::new();
            let mut bass: Vec<crate::analysis::BassOnset> = Vec::new();
            for placement in
                song.placements_in_range(synth_sequencer::Tick(start), synth_sequencer::Tick(end))
            {
                let is_drum = drum_track_ids.contains(&placement.track_id);
                let is_bass = bass_track_ids.contains(&placement.track_id);
                if !is_drum && !is_bass {
                    continue;
                }
                let Some(pattern) = song.pattern(placement.pattern_id) else {
                    continue;
                };
                let placement_start = placement.start.0;
                for n in pattern.notes() {
                    let abs_start = placement_start.saturating_add(u64::from(n.start.0));
                    if abs_start < start || abs_start >= end {
                        continue;
                    }
                    let rel = (abs_start - start) as u32;
                    let midi = n.pitch.as_midi();
                    if is_drum {
                        if matches!(
                            crate::analysis::DrumComponent::from_midi(midi),
                            crate::analysis::DrumComponent::Kick
                        ) {
                            kicks.push(crate::analysis::KickOnset { tick: rel });
                        }
                    } else {
                        let transposed = n.pitch.transpose(placement.transpose);
                        let Some(p) = transposed else {
                            continue;
                        };
                        bass.push(crate::analysis::BassOnset {
                            tick: rel,
                            midi: p.as_midi(),
                        });
                    }
                }
            }

            let length_ticks: u32 = (end - start).try_into().unwrap_or(u32::MAX);
            let ts = song.time_signature_at(synth_sequencer::Tick(start));
            (
                HarmonyScope::Arrangement {
                    start_tick: start,
                    end_tick: end,
                },
                ts,
                length_ticks,
                kicks,
                bass,
                drum_track_infos,
                bass_track_infos,
                start,
                end,
            )
        }
    };

    drop(song);

    let mut analysis =
        crate::analysis::bass_drum_lock::analyze(&kicks, &bass, length_ticks, time_sig, tolerance);
    warnings.append(&mut analysis.warnings);

    let (start_bar, start_beat) = tick_to_bar_beat_1based(start_tick, time_sig);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(end_tick, time_sig);

    let on_kick_root_name = analysis
        .bass_pitch
        .on_kick_root_pc
        .map(|pc| synth_sequencer::NoteName::from_midi(pc).to_string());

    Ok(AnalyzeBassDrumLockResult {
        scope,
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        length_ticks: analysis.length_ticks,
        length_bars: analysis.length_bars,
        time_signature_numerator: time_sig.numerator,
        time_signature_denominator: time_sig.denominator,
        drum_tracks,
        bass_tracks,
        kick_onset_count: analysis.kick_onset_count,
        bass_onset_count: analysis.bass_onset_count,
        onset_tolerance_ticks: analysis.onset_tolerance_ticks,
        alignment: BassDrumAlignment {
            matched_onsets: analysis.alignment.matched_onsets,
            kick_only: analysis.alignment.kick_only,
            bass_only: analysis.alignment.bass_only,
            lock_score: analysis.alignment.lock_score,
            coverage_score: analysis.alignment.coverage_score,
        },
        bass_pitch: BassPitchStability {
            on_kick_root_pc: analysis.bass_pitch.on_kick_root_pc,
            on_kick_root_name,
            on_kick_root_share: analysis.bass_pitch.on_kick_root_share,
            distinct_pcs_on_kick: analysis.bass_pitch.distinct_pcs_on_kick,
            distinct_pcs_total: analysis.bass_pitch.distinct_pcs_total,
            mean_bass_midi: analysis.bass_pitch.mean_bass_midi,
        },
        warnings,
    })
}

#[allow(clippy::too_many_arguments)]
fn analyze_harmonic_function_impl(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<u32>,
    arrangement_start_tick: Option<u64>,
    arrangement_end_tick: Option<u64>,
    grouping_ticks: Option<u32>,
    exclude_drums: Option<bool>,
    exclude_track_ids: Option<Vec<u16>>,
) -> Result<synth_mcp::types::AnalyzeHarmonicFunctionResult, McpBridgeError> {
    use synth_mcp::types::{
        AnalyzeHarmonicFunctionResult, ChordFunctionEvent, FunctionDistribution,
        HarmonicCadenceEvent, HarmonicCadenceKind, TensionStats,
    };

    // Reuse the harmony analyzer end-to-end so the key inference + chord
    // identification + drum-exclusion behaviour stays in lock-step with
    // analyze_harmony.
    let harmony = analyze_song_harmony(
        session,
        shared,
        pattern_id,
        arrangement_start_tick,
        arrangement_end_tick,
        grouping_ticks,
        exclude_drums,
        exclude_track_ids,
    )?;

    let key_mode = harmony
        .inferred_key
        .as_ref()
        .map(|k| crate::analysis::KeyMode::from_label(&k.mode))
        .unwrap_or(crate::analysis::KeyMode::Major);
    let tonic = harmony.inferred_key.as_ref().map(|k| k.tonic);

    let chord_inputs: Vec<crate::analysis::ChordInput> = harmony
        .chords
        .iter()
        .map(|e| crate::analysis::ChordInput {
            symbol: e.symbol.clone(),
            root: e.root,
            quality: e.quality.clone(),
            in_key: e.in_key,
        })
        .collect();

    let analysis = crate::analysis::harmonic_function::analyze(&chord_inputs, tonic, key_mode);

    let mut warnings = harmony.warnings.clone();
    warnings.extend(analysis.warnings.iter().cloned());

    let chord_events: Vec<ChordFunctionEvent> = harmony
        .chords
        .iter()
        .zip(analysis.chords.iter())
        .map(|(harmony_event, fn_event)| ChordFunctionEvent {
            symbol: fn_event.symbol.clone(),
            start_bar: harmony_event.start_bar,
            start_beat: harmony_event.start_beat,
            start_tick: harmony_event.start_tick,
            end_tick: harmony_event.end_tick,
            scale_degree: fn_event.scale_degree,
            roman_numeral: fn_event.roman_numeral.clone(),
            function: fn_event.function.as_str().to_string(),
            tension: fn_event.tension,
            in_key: fn_event.in_key,
            cadence: fn_event.cadence.map(|c| c.as_str().to_string()),
        })
        .collect();

    let cadences: Vec<HarmonicCadenceEvent> = analysis
        .cadences
        .iter()
        .map(|c| HarmonicCadenceEvent {
            chord_index: c.chord_index,
            kind: match c.kind {
                crate::analysis::CadenceKind::Authentic => HarmonicCadenceKind::Authentic,
                crate::analysis::CadenceKind::Plagal => HarmonicCadenceKind::Plagal,
                crate::analysis::CadenceKind::HalfCadence => HarmonicCadenceKind::HalfCadence,
                crate::analysis::CadenceKind::Deceptive => HarmonicCadenceKind::Deceptive,
            },
        })
        .collect();

    Ok(AnalyzeHarmonicFunctionResult {
        scope: harmony.scope,
        key: harmony.inferred_key.clone(),
        chords: chord_events,
        cadences,
        function_distribution: FunctionDistribution {
            tonic: analysis.function_distribution.tonic,
            subdominant: analysis.function_distribution.subdominant,
            dominant: analysis.function_distribution.dominant,
            other: analysis.function_distribution.other,
            chromatic: analysis.function_distribution.chromatic,
        },
        tension: TensionStats {
            mean: analysis.tension.mean,
            peak: analysis.tension.peak,
            trough: analysis.tension.trough,
            std_dev: analysis.tension.std_dev,
        },
        warnings,
    })
}

// ---------------------------------------------------------------------------
// Group C — form & motif analyzer impls
// ---------------------------------------------------------------------------

/// Shared scope resolution for the four Group C tools. Resolves to either a
/// single pattern or an arrangement range, applies drum-track filtering
/// (auto-inferred + explicit) when requested in arrangement scope, and
/// returns the scope-relative [`MelodicNote`] stream plus enough context
/// (`HarmonyScope`, `TimeSignature`, `length_ticks`, `length_bars`,
/// `start/end_tick`) to populate the wire-format header fields.
#[allow(clippy::too_many_arguments)]
fn collect_form_scope(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<u32>,
    arrangement_start_tick: Option<u64>,
    arrangement_end_tick: Option<u64>,
    exclude_drums: bool,
    exclude_track_ids: &[u16],
) -> Result<FormScopeData, McpBridgeError> {
    use crate::analysis::bar_features::MelodicNote;
    use synth_mcp::types::HarmonyScope;
    use synth_sequencer::{PatternId, SeqInstrumentId};

    let song = shared.song.read();
    let mut warnings: Vec<String> = Vec::new();

    match pattern_id {
        Some(pid) => {
            let pid_typed = PatternId(pid);
            let Some(pattern) = song.pattern(pid_typed) else {
                return Err(McpBridgeError::Other(format!("Pattern {pid} not found")));
            };
            let length_ticks = pattern.length.0;
            let ts = song.default_time_signature;
            let ticks_per_bar = ts.ticks_per_bar().max(1);
            let total_bars = length_ticks.div_ceil(ticks_per_bar).max(1);
            let notes: Vec<MelodicNote> = pattern
                .notes()
                .iter()
                .map(|n| MelodicNote {
                    track_id: 0,
                    tick: n.start.0,
                    duration_ticks: n.duration.map(|d| d.0).unwrap_or(240).max(1),
                    pitch: n.pitch,
                    velocity: n.velocity.as_f32(),
                })
                .collect();
            Ok(FormScopeData {
                scope: HarmonyScope::Pattern { pattern_id: pid },
                time_sig: ts,
                length_ticks: u64::from(length_ticks),
                total_bars,
                start_tick: 0,
                end_tick: u64::from(length_ticks),
                notes,
                warnings,
            })
        }
        None => {
            let (start, end) =
                resolve_arrangement_range(&song, arrangement_start_tick, arrangement_end_tick)?;
            let ts = song.time_signature_at(synth_sequencer::Tick(start));
            let ticks_per_bar = ts.ticks_per_bar().max(1);
            let span_ticks: u32 = (end - start).try_into().unwrap_or(u32::MAX);
            let total_bars = span_ticks.div_ceil(ticks_per_bar).max(1);

            // Build the drum-filter set (auto-inferred drums + explicit
            // exclusions).
            let mut excluded: std::collections::HashSet<synth_sequencer::TrackId> =
                exclude_track_ids
                    .iter()
                    .copied()
                    .map(synth_sequencer::TrackId)
                    .collect();
            if exclude_drums {
                let drum_instrument_ids: std::collections::HashSet<SeqInstrumentId> =
                    crate::analysis::infer_all_profiles(&song, session.state())
                        .into_iter()
                        .filter(|p| {
                            p.role.role == crate::analysis::Role::Drums && p.role.confidence >= 0.6
                        })
                        .map(|p| SeqInstrumentId(p.instrument_id))
                        .collect();
                for t in song.tracks() {
                    if let Some(seq) = t.instrument
                        && drum_instrument_ids.contains(&seq)
                    {
                        excluded.insert(t.id);
                    }
                }
            }

            let mut notes: Vec<MelodicNote> = Vec::new();
            for placement in
                song.placements_in_range(synth_sequencer::Tick(start), synth_sequencer::Tick(end))
            {
                if excluded.contains(&placement.track_id) {
                    continue;
                }
                let Some(pattern) = song.pattern(placement.pattern_id) else {
                    continue;
                };
                let placement_start = placement.start.0;
                for n in pattern.notes() {
                    let abs_start = placement_start.saturating_add(u64::from(n.start.0));
                    if abs_start < start || abs_start >= end {
                        continue;
                    }
                    let Some(pitch) = n.pitch.transpose(placement.transpose) else {
                        warnings.push(format!(
                            "Note at tick {abs_start} dropped: placement transpose out of MIDI range"
                        ));
                        continue;
                    };
                    let rel = (abs_start - start) as u32;
                    notes.push(MelodicNote {
                        track_id: placement.track_id.0,
                        tick: rel,
                        duration_ticks: n.duration.map(|d| d.0).unwrap_or(240).max(1),
                        pitch,
                        velocity: n.velocity.as_f32(),
                    });
                }
            }

            if notes.is_empty() {
                warnings.push(
                    "No melodic notes inside the analyzed scope (drums excluded by default)"
                        .to_string(),
                );
            }
            Ok(FormScopeData {
                scope: HarmonyScope::Arrangement {
                    start_tick: start,
                    end_tick: end,
                },
                time_sig: ts,
                length_ticks: end - start,
                total_bars,
                start_tick: start,
                end_tick: end,
                notes,
                warnings,
            })
        }
    }
}

struct FormScopeData {
    scope: synth_mcp::types::HarmonyScope,
    time_sig: synth_sequencer::TimeSignature,
    length_ticks: u64,
    total_bars: u32,
    start_tick: u64,
    end_tick: u64,
    notes: Vec<crate::analysis::bar_features::MelodicNote>,
    warnings: Vec<String>,
}

const FORM_SIMILARITY_DEFAULT: f32 = 0.85;
const FORM_SECTION_MIN_BARS_DEFAULT: u32 = 2;

fn section_summary_to_wire(
    s: &crate::analysis::form::SectionSummary,
) -> synth_mcp::types::SectionSpan {
    synth_mcp::types::SectionSpan {
        label: s.label.clone(),
        start_bar: s.start_bar,
        end_bar: s.end_bar,
        length_bars: s.length_bars,
        mean_notes_per_bar: s.mean_notes_per_bar,
        mean_distinct_pitch_classes: s.mean_distinct_pitch_classes,
        mean_velocity: s.mean_velocity,
        active_track_ids: s.active_track_ids.clone(),
    }
}

fn clamp_similarity(t: Option<f32>) -> f32 {
    let v = t.unwrap_or(FORM_SIMILARITY_DEFAULT);
    if v.is_nan() {
        FORM_SIMILARITY_DEFAULT
    } else {
        v.clamp(0.5, 0.999)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn analyze_arrangement_impl(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<u32>,
    arrangement_start_tick: Option<u64>,
    arrangement_end_tick: Option<u64>,
    similarity_threshold: Option<f32>,
    section_min_bars: Option<u32>,
    exclude_drums: Option<bool>,
    exclude_track_ids: Option<Vec<u16>>,
) -> Result<synth_mcp::types::AnalyzeArrangementResult, McpBridgeError> {
    use synth_mcp::types::{AnalyzeArrangementResult, BarFeatureSummary, SectionSpan};

    let exclude_drums_v = exclude_drums.unwrap_or(true);
    let exclude_track_ids_v = exclude_track_ids.unwrap_or_default();
    let scope_data = collect_form_scope(
        session,
        shared,
        pattern_id,
        arrangement_start_tick,
        arrangement_end_tick,
        exclude_drums_v,
        &exclude_track_ids_v,
    )?;

    let threshold = clamp_similarity(similarity_threshold);
    let min_bars = section_min_bars
        .unwrap_or(FORM_SECTION_MIN_BARS_DEFAULT)
        .max(1);

    let analysis = crate::analysis::form::analyze_form(
        &scope_data.notes,
        scope_data.time_sig,
        scope_data.total_bars,
        threshold,
        min_bars,
    );

    let (start_bar, start_beat) =
        tick_to_bar_beat_1based(scope_data.start_tick, scope_data.time_sig);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(scope_data.end_tick, scope_data.time_sig);

    let bars: Vec<BarFeatureSummary> = analysis
        .bars
        .iter()
        .map(|b| BarFeatureSummary {
            bar: b.bar,
            note_count: b.note_count,
            distinct_pitch_classes: b.distinct_pitch_classes,
            dominant_pitch_class: b.dominant_pitch_class,
            mean_velocity: b.mean_velocity,
            active_track_ids: b.active_track_ids.clone(),
        })
        .collect();

    let sections: Vec<SectionSpan> = analysis
        .sections
        .iter()
        .map(section_summary_to_wire)
        .collect();

    let mut warnings = scope_data.warnings;
    if scope_data.total_bars < 2 {
        warnings.push("Scope is shorter than 2 bars — section clustering skipped".to_string());
    }

    Ok(AnalyzeArrangementResult {
        scope: scope_data.scope,
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        length_ticks: scope_data.length_ticks,
        length_bars: scope_data.total_bars,
        time_signature_numerator: scope_data.time_sig.numerator,
        time_signature_denominator: scope_data.time_sig.denominator,
        similarity_threshold: threshold,
        bars,
        sections,
        distinct_section_count: analysis.distinct_section_count,
        warnings,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn analyze_form_map_impl(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<u32>,
    arrangement_start_tick: Option<u64>,
    arrangement_end_tick: Option<u64>,
    similarity_threshold: Option<f32>,
    section_min_bars: Option<u32>,
    exclude_drums: Option<bool>,
    exclude_track_ids: Option<Vec<u16>>,
) -> Result<synth_mcp::types::AnalyzeFormMapResult, McpBridgeError> {
    use synth_mcp::types::{AnalyzeFormMapResult, SectionSpan};

    let exclude_drums_v = exclude_drums.unwrap_or(true);
    let exclude_track_ids_v = exclude_track_ids.unwrap_or_default();
    let scope_data = collect_form_scope(
        session,
        shared,
        pattern_id,
        arrangement_start_tick,
        arrangement_end_tick,
        exclude_drums_v,
        &exclude_track_ids_v,
    )?;
    let threshold = clamp_similarity(similarity_threshold);
    let min_bars = section_min_bars
        .unwrap_or(FORM_SECTION_MIN_BARS_DEFAULT)
        .max(1);

    let analysis = crate::analysis::form::analyze_form(
        &scope_data.notes,
        scope_data.time_sig,
        scope_data.total_bars,
        threshold,
        min_bars,
    );

    let (start_bar, start_beat) =
        tick_to_bar_beat_1based(scope_data.start_tick, scope_data.time_sig);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(scope_data.end_tick, scope_data.time_sig);

    let sections: Vec<SectionSpan> = analysis
        .sections
        .iter()
        .map(section_summary_to_wire)
        .collect();

    Ok(AnalyzeFormMapResult {
        scope: scope_data.scope,
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        length_bars: scope_data.total_bars,
        time_signature_numerator: scope_data.time_sig.numerator,
        time_signature_denominator: scope_data.time_sig.denominator,
        similarity_threshold: threshold,
        bar_labels: analysis.bar_labels,
        form_string: analysis.form_string,
        sections,
        distinct_section_count: analysis.distinct_section_count,
        warnings: scope_data.warnings,
    })
}

const MOTIF_MIN_LEN_DEFAULT: u8 = 3;
const MOTIF_MAX_LEN_DEFAULT: u8 = 6;
const MOTIF_MIN_COUNT_DEFAULT: u32 = 3;
const MOTIF_TOP_N_DEFAULT: u32 = 10;
const MOTIF_LEN_HARD_CAP: u8 = 12;

fn clamp_motif_lengths(min_len: Option<u8>, max_len: Option<u8>) -> (u8, u8) {
    let lo = min_len
        .unwrap_or(MOTIF_MIN_LEN_DEFAULT)
        .clamp(2, MOTIF_LEN_HARD_CAP);
    let mut hi = max_len
        .unwrap_or(MOTIF_MAX_LEN_DEFAULT)
        .min(MOTIF_LEN_HARD_CAP);
    if hi < lo {
        hi = lo;
    }
    (lo, hi)
}

fn motif_occurrences_to_wire(
    hits: &[crate::analysis::motifs::MotifHit],
    scope_start_tick: u64,
    time_sig: synth_sequencer::TimeSignature,
) -> Vec<synth_mcp::types::MotifOccurrence> {
    hits.iter()
        .map(|h| {
            let abs_tick = scope_start_tick + u64::from(h.start_tick);
            let (bar, beat) = tick_to_bar_beat_1based(abs_tick, time_sig);
            synth_mcp::types::MotifOccurrence {
                track_id: h.track_id,
                start_tick: abs_tick,
                start_bar: bar,
                start_beat: beat,
                first_pitch: h.first_pitch,
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn find_motifs_impl(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<u32>,
    arrangement_start_tick: Option<u64>,
    arrangement_end_tick: Option<u64>,
    min_interval_length: Option<u8>,
    max_interval_length: Option<u8>,
    min_count: Option<u32>,
    top_n: Option<u32>,
    exclude_drums: Option<bool>,
    exclude_track_ids: Option<Vec<u16>>,
) -> Result<synth_mcp::types::FindMotifsResult, McpBridgeError> {
    use synth_mcp::types::{FindMotifsResult, MotifEntry};

    let exclude_drums_v = exclude_drums.unwrap_or(true);
    let exclude_track_ids_v = exclude_track_ids.unwrap_or_default();
    let scope_data = collect_form_scope(
        session,
        shared,
        pattern_id,
        arrangement_start_tick,
        arrangement_end_tick,
        exclude_drums_v,
        &exclude_track_ids_v,
    )?;

    let (min_len, max_len) = clamp_motif_lengths(min_interval_length, max_interval_length);
    let min_count_v = min_count.unwrap_or(MOTIF_MIN_COUNT_DEFAULT).max(2);
    let top_n_v = top_n.unwrap_or(MOTIF_TOP_N_DEFAULT).max(1);

    let motifs = crate::analysis::motifs::find_motifs(
        &scope_data.notes,
        min_len,
        max_len,
        min_count_v,
        top_n_v,
    );

    let (start_bar, start_beat) =
        tick_to_bar_beat_1based(scope_data.start_tick, scope_data.time_sig);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(scope_data.end_tick, scope_data.time_sig);

    let wire_motifs: Vec<MotifEntry> = motifs
        .iter()
        .map(|m| MotifEntry {
            length: m.length,
            intervals: m.intervals.clone(),
            count: m.count(),
            occurrences: motif_occurrences_to_wire(
                &m.occurrences,
                scope_data.start_tick,
                scope_data.time_sig,
            ),
        })
        .collect();

    let mut warnings = scope_data.warnings;
    if scope_data.notes.len() < (max_len as usize) + 1 {
        warnings.push(format!(
            "Only {} melodic notes in scope — too few to form motifs of length {}",
            scope_data.notes.len(),
            max_len + 1
        ));
    }

    Ok(FindMotifsResult {
        scope: scope_data.scope,
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        min_interval_length: min_len,
        max_interval_length: max_len,
        min_count: min_count_v,
        total_notes: scope_data.notes.len() as u32,
        motifs: wire_motifs,
        warnings,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn analyze_hook_strength_impl(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<u32>,
    arrangement_start_tick: Option<u64>,
    arrangement_end_tick: Option<u64>,
    min_interval_length: Option<u8>,
    min_count: Option<u32>,
    exclude_drums: Option<bool>,
    exclude_track_ids: Option<Vec<u16>>,
) -> Result<synth_mcp::types::AnalyzeHookStrengthResult, McpBridgeError> {
    use synth_mcp::types::{AnalyzeHookStrengthResult, MotifEntry};

    let exclude_drums_v = exclude_drums.unwrap_or(true);
    let exclude_track_ids_v = exclude_track_ids.unwrap_or_default();
    let scope_data = collect_form_scope(
        session,
        shared,
        pattern_id,
        arrangement_start_tick,
        arrangement_end_tick,
        exclude_drums_v,
        &exclude_track_ids_v,
    )?;

    let min_len = min_interval_length
        .unwrap_or(MOTIF_MIN_LEN_DEFAULT)
        .clamp(2, MOTIF_LEN_HARD_CAP);
    let min_count_v = min_count.unwrap_or(MOTIF_MIN_COUNT_DEFAULT).max(2);
    // Hook-strength always sweeps up to the hard cap so a long but rare
    // motif can still win — the caller controls the *minimum*, not the
    // maximum length considered.
    let motifs = crate::analysis::motifs::find_motifs(
        &scope_data.notes,
        min_len,
        MOTIF_LEN_HARD_CAP,
        min_count_v,
        50,
    );
    let analysis =
        crate::analysis::motifs::hook_strength(&scope_data.notes, &motifs, min_len, min_count_v);

    let (start_bar, start_beat) =
        tick_to_bar_beat_1based(scope_data.start_tick, scope_data.time_sig);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(scope_data.end_tick, scope_data.time_sig);

    let strongest_wire = analysis.strongest.map(|m| MotifEntry {
        length: m.length,
        intervals: m.intervals.clone(),
        count: m.count(),
        occurrences: motif_occurrences_to_wire(
            &m.occurrences,
            scope_data.start_tick,
            scope_data.time_sig,
        ),
    });

    Ok(AnalyzeHookStrengthResult {
        scope: scope_data.scope,
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        total_notes: scope_data.notes.len() as u32,
        hook_score: analysis.score,
        coverage_ratio: analysis.coverage,
        strongest_motif: strongest_wire,
        min_interval_length: min_len,
        min_count: min_count_v,
        warnings: scope_data.warnings,
    })
}

/// Default mix-bus render duration when the caller leaves it unspecified.
const DEFAULT_MIX_BUS_SECONDS: f32 = 10.0;

/// Convert a `MixAnalysis` into the wire-format `MixBusMetrics`.
fn mix_metrics_from_analysis(
    analysis: &crate::audio::mix_analysis::MixAnalysis,
    sample_rate: u32,
    duration_seconds: f32,
) -> MixBusMetrics {
    MixBusMetrics {
        sample_rate,
        duration_seconds,
        peak: analysis.peak,
        peak_dbfs: analysis.peak_dbfs,
        peak_left: analysis.peak_left,
        peak_right: analysis.peak_right,
        true_peak: analysis.true_peak,
        true_peak_dbtp: analysis.true_peak_dbtp,
        rms: analysis.rms,
        rms_dbfs: analysis.rms_dbfs,
        crest_factor_db: analysis.crest_factor_db,
        lufs_integrated: analysis.lufs_integrated,
        lufs_momentary_max: analysis.lufs_momentary_max,
        lufs_short_term_max: analysis.lufs_short_term_max,
        energy_bands: analysis.energy_bands.into(),
        stereo_correlation: analysis.stereo_correlation,
        mid_rms: analysis.mid_rms,
        side_rms: analysis.side_rms,
        stereo_width: analysis.stereo_width,
        mono_compat: analysis.mono_compat,
        clipped_samples: analysis.clipped_samples,
    }
}

/// `analyze_mix_bus` bridge implementation. Renders `duration_seconds` of the
/// master bus offline starting at `start_tick` (default 0).
fn analyze_mix_bus_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    duration_seconds: f32,
    start_tick: Option<u64>,
) -> Result<AnalyzeMixBusResult, McpBridgeError> {
    let dur = if duration_seconds.is_nan() || duration_seconds <= 0.0 {
        DEFAULT_MIX_BUS_SECONDS
    } else {
        duration_seconds
    };
    if dur > 300.0 {
        return Err(McpBridgeError::Other(format!(
            "duration_seconds {dur} exceeds the 300-second maximum"
        )));
    }
    let start = start_tick.unwrap_or(0);

    // Convert the requested duration into a tick offset using the song's
    // tempo so the renderer can do its own tick-range render.
    let end = {
        let song = shared.song.read();
        let start_seconds = song.tick_to_seconds(synth_sequencer::Tick(start));
        let target_seconds = start_seconds + f64::from(dur);
        song.seconds_to_tick(target_seconds).0
    };
    if end <= start {
        return Err(McpBridgeError::Other(
            "Requested duration resolves to zero song ticks — check tempo".to_string(),
        ));
    }

    let rendered = crate::audio::arrangement_render::render_arrangement_to_buffer(
        session,
        sample_library,
        shared,
        start,
        end,
    )?;
    let analysis =
        crate::audio::mix_analysis::analyze_mix_buffer(&rendered.samples, rendered.sample_rate);
    let metrics =
        mix_metrics_from_analysis(&analysis, rendered.sample_rate, rendered.duration_seconds);
    let ts = shared
        .song
        .read()
        .time_signature_at(synth_sequencer::Tick(rendered.start_tick));
    let (start_bar, start_beat) = tick_to_bar_beat_1based(rendered.start_tick, ts);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(rendered.end_tick, ts);
    Ok(AnalyzeMixBusResult {
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        start_tick: rendered.start_tick,
        end_tick: rendered.end_tick,
        metrics,
        warnings: rendered.warnings,
    })
}

/// `analyze_section` bridge implementation. Renders an explicit
/// `[start_tick, end_tick)` arrangement range offline and returns
/// mix-bus metrics. When `include_per_track` is true, also re-renders each
/// audible track soloed in turn and returns per-track contribution metrics.
#[doc(hidden)]
pub fn analyze_section_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    start_tick: u64,
    end_tick: u64,
    include_per_track: Option<bool>,
) -> Result<AnalyzeSectionResult, McpBridgeError> {
    let rendered = crate::audio::arrangement_render::render_arrangement_to_buffer(
        session,
        sample_library,
        shared,
        start_tick,
        end_tick,
    )?;
    let analysis =
        crate::audio::mix_analysis::analyze_mix_buffer(&rendered.samples, rendered.sample_rate);
    let metrics =
        mix_metrics_from_analysis(&analysis, rendered.sample_rate, rendered.duration_seconds);
    let ts = shared
        .song
        .read()
        .time_signature_at(synth_sequencer::Tick(rendered.start_tick));
    let (start_bar, start_beat) = tick_to_bar_beat_1based(rendered.start_tick, ts);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(rendered.end_tick, ts);

    let mut warnings = rendered.warnings;
    let per_track = if include_per_track.unwrap_or(false) {
        render_per_track_contributions(
            session,
            sample_library,
            shared,
            start_tick,
            end_tick,
            &mut warnings,
        )?
    } else {
        Vec::new()
    };

    Ok(AnalyzeSectionResult {
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        start_tick: rendered.start_tick,
        end_tick: rendered.end_tick,
        metrics,
        per_track,
        warnings,
    })
}

/// Resolve which tracks have placements overlapping the section and
/// re-render each one soloed against a cloned song. Warnings from each
/// soloed render are accumulated into `warnings`.
fn render_per_track_contributions(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    start_tick: u64,
    end_tick: u64,
    warnings: &mut Vec<String>,
) -> Result<Vec<synth_mcp::types::TrackContribution>, McpBridgeError> {
    use synth_sequencer::TrackId;

    struct TargetMeta {
        track_id: TrackId,
        name: String,
        instrument_id: Option<u16>,
    }

    let (targets, base_song) = {
        let song = shared.song.read();
        let any_solo = song.any_solo();
        let mut covered: std::collections::HashSet<TrackId> = std::collections::HashSet::new();
        for placement in song.placements_in_range(
            synth_sequencer::Tick(start_tick),
            synth_sequencer::Tick(end_tick),
        ) {
            covered.insert(placement.track_id);
        }
        let mut targets: Vec<TargetMeta> = covered
            .into_iter()
            .filter_map(|tid| {
                let t = song.track(tid)?;
                t.is_audible(any_solo).then_some(TargetMeta {
                    track_id: tid,
                    name: t.name.clone(),
                    instrument_id: t.instrument.map(|s| s.0),
                })
            })
            .collect();
        targets.sort_by_key(|t| t.track_id.0);
        (targets, song.clone())
    };

    if targets.is_empty() {
        warnings.push(
            "include_per_track requested but no audible tracks overlap the section".to_string(),
        );
        return Ok(Vec::new());
    }

    // Snapshot each instrument's pan + volume so we can analytically reverse
    // their attenuation when computing `pre_master_peak`. The realtime engine
    // applies pan-law and per-instrument volume at the mix-down stage; the
    // soloed render already contains the resulting attenuated signal, so a
    // single division by (volume × max_pan_gain) on the loud-channel peak
    // recovers the patch's internal-signal peak. Saves a second render per
    // track vs. re-rendering with pan/volume overridden.
    let instrument_gains: std::collections::HashMap<u16, (Gain, BipolarValue)> = {
        let snapshots = session.state().instrument_snapshots.read();
        snapshots
            .iter()
            .map(|s| (s.id.as_u64() as u16, (s.volume, s.pan)))
            .collect()
    };

    // Determinism for the parallel renders rests on the §8.1 Round-2 fixes:
    // BTreeMap ordering in `synth_engine::graph` + `fastrand` reseed per
    // `render_range`. Engine-level setup warnings reflect the live session
    // state (not the target solo flag), so they are identical for every
    // worker — `enumerate` lets the idx == 0 worker push them and the rest
    // discard.
    use rayon::prelude::*;

    let render_pairs: Result<
        Vec<(synth_mcp::types::TrackContribution, Vec<String>)>,
        McpBridgeError,
    > = targets
        .par_iter()
        .enumerate()
        .map(|(idx, target)| -> Result<_, McpBridgeError> {
            let (mut engine_session, setup_warnings) =
                crate::audio::arrangement_render::OfflineEngineSession::new(
                    session,
                    sample_library,
                )?;

            let mut song_clone = base_song.clone();
            song_clone.set_solo_only(target.track_id);
            let song_arc = std::sync::Arc::new(parking_lot::RwLock::new(song_clone));

            let rendered = engine_session.render_range(&song_arc, start_tick, end_tick)?;
            let mut per_target_warnings: Vec<String> =
                if idx == 0 { setup_warnings } else { Vec::new() };
            for w in &rendered.warnings {
                per_target_warnings.push(format!("{}({}): {w}", target.name, target.track_id.0));
            }

            let analysis = crate::audio::mix_analysis::analyze_mix_buffer(
                &rendered.samples,
                rendered.sample_rate,
            );
            let metrics = mix_metrics_from_analysis(
                &analysis,
                rendered.sample_rate,
                rendered.duration_seconds,
            );
            let (pre_master_peak, pre_master_peak_dbfs) = pre_master_peak_for(
                target
                    .instrument_id
                    .and_then(|id| instrument_gains.get(&id)),
                analysis.peak_left,
                analysis.peak_right,
            );

            Ok((
                synth_mcp::types::TrackContribution {
                    track_id: target.track_id.0,
                    track_name: target.name.clone(),
                    instrument_id: target.instrument_id,
                    metrics,
                    pre_master_peak,
                    pre_master_peak_dbfs,
                    rms_share: 0.0,
                },
                per_target_warnings,
            ))
        })
        .collect();

    let mut contributions: Vec<synth_mcp::types::TrackContribution> =
        Vec::with_capacity(targets.len());
    for (c, ws) in render_pairs? {
        contributions.push(c);
        warnings.extend(ws);
    }

    let total_rms: f32 = contributions.iter().map(|c| c.metrics.rms).sum();
    if total_rms > 0.0 {
        for c in contributions.iter_mut() {
            c.rms_share = (c.metrics.rms / total_rms).clamp(0.0, 1.0);
        }
    }

    Ok(contributions)
}

/// Reverse the engine's `volume × pan_gain` attenuation to recover the
/// instrument's pre-mix peak from the soloed render's per-channel peaks.
///
/// Constant-power pan-law gives `(gL, gR) = Gain::from_pan(pan)`; each channel
/// in the soloed render is `internal × volume × g_channel`. Dividing the
/// per-channel peak by its own gain reverses both effects in one step; we
/// take the larger of the two to handle hard-panned signals where one channel
/// is silent.
///
/// Returns `(linear, dBFS)`. When the instrument's gains are unknown (no
/// matching snapshot) or fully zero, falls back to the larger of the two raw
/// channel peaks so the caller still sees a usable lower bound.
fn pre_master_peak_for(
    gains: Option<&(Gain, BipolarValue)>,
    peak_left: f32,
    peak_right: f32,
) -> (f32, f32) {
    let raw_peak = peak_left.max(peak_right);
    let restored = match gains {
        Some((volume, pan)) => {
            let v = volume.as_f32();
            let (gl, gr) = Gain::from_pan(*pan);
            let combined_l = v * gl.as_f32();
            let combined_r = v * gr.as_f32();
            let left = if combined_l > 1e-6 {
                peak_left / combined_l
            } else {
                0.0
            };
            let right = if combined_r > 1e-6 {
                peak_right / combined_r
            } else {
                0.0
            };
            let max = left.max(right);
            if max > 0.0 { max } else { raw_peak }
        }
        None => raw_peak,
    };
    (restored, crate::audio::mix_analysis::lin_to_db(restored))
}

/// Frequency band definitions for `analyze_masking_matrix`, matching the
/// `AnalyzeEnergyBands` split used by `analyze_mix_buffer`.
const MASKING_BAND_DEFS: &[(&str, f32, f32)] = &[
    ("sub", 0.0, 100.0),
    ("low", 100.0, 500.0),
    ("mid", 500.0, 2000.0),
    ("high", 2000.0, 20_000.0),
];

/// Minimum overlap energy (linear RMS) at which a band is loud enough to
/// warrant a textual hint. Below this the bands are reported but no hint is
/// generated — every multi-track section would otherwise produce noise.
const MASKING_HINT_MIN_ENERGY: f32 = 0.01;

/// Dominance margin in dB above which we call one track a "masker" of the
/// other on the worst-overlap band. Below this the pair is reported as an
/// even competition.
const MASKING_DOMINANCE_DB_THRESHOLD: f32 = 6.0;

fn masking_band_overlap(name: &str, lo: f32, hi: f32, a: f32, b: f32) -> BandOverlap {
    let lower = a.min(b).max(0.0);
    let upper = a.max(b).max(0.0);
    let dominance_db = if upper < 1e-9 {
        0.0
    } else if lower < 1e-9 {
        200.0
    } else {
        crate::audio::mix_analysis::lin_to_db(upper / lower).min(200.0)
    };
    BandOverlap {
        band: name.to_string(),
        freq_low_hz: lo,
        freq_high_hz: hi,
        track_a_energy: a,
        track_b_energy: b,
        overlap_energy: lower,
        dominance_db,
    }
}

fn masking_pair_bands(
    a: &synth_mcp::types::AnalyzeEnergyBands,
    b: &synth_mcp::types::AnalyzeEnergyBands,
) -> Vec<BandOverlap> {
    let aa = [a.sub, a.low, a.mid, a.high];
    let bb = [b.sub, b.low, b.mid, b.high];
    MASKING_BAND_DEFS
        .iter()
        .enumerate()
        .map(|(i, (n, lo, hi))| masking_band_overlap(n, *lo, *hi, aa[i], bb[i]))
        .collect()
}

fn masking_conflict_score(bands: &[BandOverlap]) -> f32 {
    let overlap_total: f32 = bands.iter().map(|b| b.overlap_energy).sum();
    let max_total: f32 = bands
        .iter()
        .map(|b| b.track_a_energy.max(b.track_b_energy))
        .sum();
    if max_total > 1e-9 {
        (overlap_total / max_total).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Returns `(hint, dominant_track_id)` for the worst-overlap band, or
/// `(None, None)` when no band crosses `MASKING_HINT_MIN_ENERGY`. The
/// `dominant_track_id` is set only when the margin exceeds
/// `MASKING_DOMINANCE_DB_THRESHOLD`.
fn masking_hint_for_pair(
    bands: &[BandOverlap],
    a_name: &str,
    a_id: u16,
    b_name: &str,
    b_id: u16,
) -> (Option<String>, Option<u16>) {
    let Some(worst) = bands.iter().max_by(|x, y| {
        x.overlap_energy
            .partial_cmp(&y.overlap_energy)
            .unwrap_or(std::cmp::Ordering::Equal)
    }) else {
        return (None, None);
    };
    if worst.overlap_energy < MASKING_HINT_MIN_ENERGY {
        return (None, None);
    }
    let band_label = format!(
        "{} ({:.0}-{:.0} Hz)",
        worst.band, worst.freq_low_hz, worst.freq_high_hz
    );
    if worst.dominance_db >= MASKING_DOMINANCE_DB_THRESHOLD {
        let (masker_name, masker_id, masked_name, masked_id) =
            if worst.track_a_energy >= worst.track_b_energy {
                (a_name, a_id, b_name, b_id)
            } else {
                (b_name, b_id, a_name, a_id)
            };
        (
            Some(format!(
                "{masker_name}({masker_id}) masks {masked_name}({masked_id}) in {band_label}"
            )),
            Some(masker_id),
        )
    } else {
        (
            Some(format!(
                "{a_name}({a_id}) and {b_name}({b_id}) compete in {band_label}"
            )),
            None,
        )
    }
}

fn build_masking_pairs(contributions: &[synth_mcp::types::TrackContribution]) -> Vec<MaskingPair> {
    let n = contributions.len();
    let mut pairs: Vec<MaskingPair> = Vec::with_capacity(n.saturating_mul(n.saturating_sub(1)) / 2);
    for i in 0..n {
        for j in (i + 1)..n {
            let (lo, hi) = if contributions[i].track_id <= contributions[j].track_id {
                (&contributions[i], &contributions[j])
            } else {
                (&contributions[j], &contributions[i])
            };
            let bands = masking_pair_bands(&lo.metrics.energy_bands, &hi.metrics.energy_bands);
            let conflict_score = masking_conflict_score(&bands);
            let (hint, dominant_track_id) = masking_hint_for_pair(
                &bands,
                &lo.track_name,
                lo.track_id,
                &hi.track_name,
                hi.track_id,
            );
            pairs.push(MaskingPair {
                track_a_id: lo.track_id,
                track_a_name: lo.track_name.clone(),
                track_b_id: hi.track_id,
                track_b_name: hi.track_name.clone(),
                bands,
                conflict_score,
                dominant_track_id,
                hint,
            });
        }
    }
    pairs.sort_by(|x, y| {
        y.conflict_score
            .partial_cmp(&x.conflict_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    pairs
}

/// `analyze_masking_matrix` bridge implementation. Renders every audible
/// track soloed, then computes pairwise per-band overlap and an optional
/// textual hint. Returns pairs sorted by descending `conflict_score`.
#[doc(hidden)]
pub fn analyze_masking_matrix_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    start_tick: u64,
    end_tick: u64,
) -> Result<AnalyzeMaskingMatrixResult, McpBridgeError> {
    if end_tick <= start_tick {
        return Err(McpBridgeError::Other(format!(
            "Section range invalid: end_tick ({end_tick}) must be greater than start_tick ({start_tick})"
        )));
    }

    let mut warnings: Vec<String> = Vec::new();
    let contributions = render_per_track_contributions(
        session,
        sample_library,
        shared,
        start_tick,
        end_tick,
        &mut warnings,
    )?;

    if contributions.len() < 2 {
        warnings.push(format!(
            "analyze_masking_matrix needs at least 2 audible tracks in the section; got {}",
            contributions.len()
        ));
    }

    let pairs = build_masking_pairs(&contributions);

    let ts = shared
        .song
        .read()
        .time_signature_at(synth_sequencer::Tick(start_tick));
    let (start_bar, start_beat) = tick_to_bar_beat_1based(start_tick, ts);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(end_tick, ts);

    Ok(AnalyzeMaskingMatrixResult {
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        start_tick,
        end_tick,
        track_count: contributions.len() as u32,
        pairs,
        warnings,
    })
}

/// Convert the internal `InstrumentProfile` to the MCP-wire form.
/// The two structs have identical serde shapes (snake_case enum variants),
/// so we go through `serde_json` for the enum→string conversion rather than
/// hand-maintain a parallel `as_str` impl on every enum.
fn profile_to_result(
    profile: crate::analysis::InstrumentProfile,
) -> synth_mcp::types::InstrumentProfileResult {
    use synth_mcp::types::{InstrumentProfileResult, ProfileSignalResult, RoleInferenceResult};

    fn enum_to_str<T: serde::Serialize>(v: &T) -> String {
        match serde_json::to_value(v) {
            Ok(serde_json::Value::String(s)) => s,
            _ => String::new(),
        }
    }

    InstrumentProfileResult {
        instrument_id: profile.instrument_id,
        instrument_name: profile.instrument_name,
        role: RoleInferenceResult {
            role: enum_to_str(&profile.role.role),
            confidence: profile.role.confidence,
            signals: profile
                .role
                .signals
                .into_iter()
                .map(|s| ProfileSignalResult {
                    axis: s.axis.as_str().to_string(),
                    detail: s.detail.to_string(),
                })
                .collect(),
        },
        envelope_shape: enum_to_str(&profile.envelope_shape),
        pitch_role: enum_to_str(&profile.pitch_role),
        register: enum_to_str(&profile.register),
        texture: enum_to_str(&profile.texture),
    }
}

/// Pure analysis pass over an already-rendered audio buffer. Split out from
/// [`analyze_rendered_note`] so tests can drive analysis with synthesized
/// signals (anti-phase tones, clipped tails, etc.) without spinning up the
/// full audio engine.
#[doc(hidden)]
pub fn analyze_rendered_buffer(
    rendered: &crate::audio::preview::RenderedNote,
    note: u8,
    velocity: u8,
    duration_ms: u32,
    expected_note: Option<u8>,
) -> synth_mcp::types::AnalyzeNoteResult {
    use crate::audio::analysis;
    use synth_core::types::StereoSample;

    // Sanitize non-finite samples up-front. If a voice or effect module
    // misbehaves and produces NaN/±∞, every downstream metric (peak/RMS/DC,
    // FFT, etc.) silently returns NaN, which the JSON serializer then
    // turns into `null`. Replacing non-finite samples with 0 here keeps the
    // metrics meaningful — `clipped_samples` still records the saturated
    // range, so a runaway DSP doesn't disappear from the report.
    let rendered_samples_owned: Vec<f32>;
    let samples_slice: &[f32] = if rendered.samples.iter().all(|s| s.is_finite()) {
        &rendered.samples
    } else {
        rendered_samples_owned = rendered
            .samples
            .iter()
            .map(|&s| if s.is_finite() { s } else { 0.0 })
            .collect();
        &rendered_samples_owned
    };

    // Mix stereo-interleaved buffer down to mono for time-domain metrics
    // (peak, RMS, DC). Per-channel and mid/side decompositions below capture
    // stereo-specific behavior. Spectral / pitch analysis uses a separate
    // `analysis_signal` (see Bug 5) so anti-phase tonal content does not
    // cancel in the mono mix.
    let channels = usize::from(rendered.channels);
    let mono: Vec<f32> = match channels {
        0 => Vec::new(),
        1 => samples_slice.to_vec(),
        2 => {
            let frames = samples_slice.len() / 2;
            StereoSample::iter_frames(samples_slice, frames)
                .map(StereoSample::to_mono)
                .collect()
        }
        n => samples_slice
            .chunks_exact(n)
            .map(|frame| frame.iter().sum::<f32>() / n as f32)
            .collect(),
    };
    let sample_rate = rendered.sample_rate;

    // Per-channel decomposition for stereo signals. We compute these from
    // the (sanitized) interleaved buffer so anti-phase content cannot cancel.
    let (left_samples, right_samples): (Vec<f32>, Vec<f32>) = if rendered.channels >= 2 {
        let frames = samples_slice.len() / channels;
        let mut l = Vec::with_capacity(frames);
        let mut r = Vec::with_capacity(frames);
        for frame in samples_slice.chunks_exact(channels) {
            l.push(frame[0]);
            r.push(frame[1]);
        }
        (l, r)
    } else {
        (Vec::new(), Vec::new())
    };
    let (mid_samples, side_samples): (Vec<f32>, Vec<f32>) = if rendered.channels >= 2 {
        let len = left_samples.len();
        let mut mid = Vec::with_capacity(len);
        let mut side = Vec::with_capacity(len);
        for i in 0..len {
            mid.push((left_samples[i] + right_samples[i]) * 0.5);
            side.push((left_samples[i] - right_samples[i]) * 0.5);
        }
        (mid, side)
    } else {
        (Vec::new(), Vec::new())
    };

    // Bug 5 fix: spectral / pitch / energy analysis runs on a "phase-robust"
    // signal that survives anti-phase stereo content. Per-sample max(|L|, |R|)
    // preserves energy regardless of channel polarity (a 180°-out tone has
    // m≈0 in the mono mix but max-abs equals the original amplitude on every
    // sample). For mono input the signal is identical to `mono`.
    let analysis_signal: Vec<f32> = if rendered.channels >= 2 {
        let frames = samples_slice.len() / channels;
        let mut sig = Vec::with_capacity(frames);
        for frame in samples_slice.chunks_exact(channels) {
            // Preserve mono sign (so DC / odd-symmetry detection stays sane)
            // by picking the channel with the larger magnitude.
            let l = frame[0];
            let r = frame[1];
            let v = if l.abs() >= r.abs() { l } else { r };
            sig.push(v);
        }
        sig
    } else {
        mono.clone()
    };

    // Sample windows for the spectrum snapshots. Attack: 50–150 ms in,
    // capturing the onset transient. Sustain: actual middle of the held
    // portion, NOT the last 100 ms (which we used to do incorrectly).
    // Release: starts 25 ms after note-off — that's where the decay tail
    // and any release-trigger transient sits, instead of "last 100 ms of
    // total render" (which often missed the note-off entirely).
    let total_samples = mono.len();
    let note_samples = (f64::from(duration_ms) / 1000.0 * f64::from(sample_rate)) as usize;
    let attack_start = (0.05 * f64::from(sample_rate)) as usize;
    let attack_window = (0.10 * f64::from(sample_rate)) as usize;
    let nominal_window = attack_window;
    // True midpoint of the held note, biased so the window stays inside the
    // hold even for very short notes. Sustain is allowed to slide back to fit
    // a full window (it stays inside the held portion either way).
    let sustain_center = note_samples / 2;
    let sustain_start_target = sustain_center.saturating_sub(nominal_window / 2);
    let sustain_max_start = total_samples.saturating_sub(nominal_window.min(total_samples));
    let sustain_start = sustain_start_target.min(sustain_max_start);
    let sustain_end = sustain_start
        .saturating_add(nominal_window)
        .min(total_samples);
    // Bug 4 fix: anchor release relative to the actual note-off frame from
    // the render (see RenderedNote::note_off_frame), with a 25 ms
    // post-note-off offset. Never let the window slide BACKWARD past
    // note_off+offset — that would pull sustain audio into the release slice
    // on short tails. Instead, allow a shorter slice (the analysis helpers
    // tolerate any length) and let the slice end at total_samples.
    let release_offset_samples = (0.025 * f64::from(sample_rate)) as usize;
    let note_off_sample = rendered.note_off_frame as usize;
    let release_start = note_off_sample
        .saturating_add(release_offset_samples)
        .min(total_samples);
    let release_end = release_start
        .saturating_add(nominal_window)
        .min(total_samples);

    let attack_clamped = attack_start.min(total_samples);
    let attack_end = attack_clamped
        .saturating_add(nominal_window)
        .min(total_samples);

    let attack_slice = analysis_signal
        .get(attack_clamped..attack_end)
        .unwrap_or(&[]);
    let sustain_slice = analysis_signal
        .get(sustain_start..sustain_end)
        .unwrap_or(&[]);
    let release_slice = analysis_signal
        .get(release_start..release_end)
        .unwrap_or(&[]);

    let sr_f32 = sample_rate as f32;
    let attack_window_start_ms = attack_clamped as f32 * 1000.0 / sr_f32;
    let sustain_window_start_ms = sustain_start as f32 * 1000.0 / sr_f32;
    let release_window_start_ms = release_start as f32 * 1000.0 / sr_f32;

    let to_peaks =
        |peaks: Vec<analysis::SpectrumPeak>| -> Vec<synth_mcp::types::AnalyzeSpectrumPeak> {
            peaks.into_iter().map(Into::into).collect()
        };

    // Anchor pitch metrics. When the caller provides `expected_note`, narrow
    // the fundamental search to ±tritone (1.4× either way) so the detector
    // ignores sub-octave dominance and harmonic clutter. Otherwise sweep the
    // full audible-bass range and report whatever peak is loudest.
    let anchor_note = expected_note.unwrap_or(rendered.effective_note.as_u8());
    let expected_fundamental = synth_core::types::Hertz::from_midi(anchor_note);
    let expected_fundamental_hz = expected_fundamental.as_f32();
    let (search_min, search_max) = match expected_note {
        Some(_) if expected_fundamental_hz > 0.0 => {
            let lo = expected_fundamental_hz / std::f32::consts::SQRT_2;
            let hi = expected_fundamental_hz * std::f32::consts::SQRT_2;
            (lo.max(20.0), hi.min(20_000.0))
        }
        _ => (25.0, 5500.0),
    };

    // Pitch analysis uses `analysis_signal` so anti-phase tonal content
    // does not cancel — see Bug 5 note above.
    let pitch_slice = analysis_signal
        .get(attack_start..note_samples.min(total_samples))
        .unwrap_or(&analysis_signal);
    let (fundamental_hz, pitch_confidence) = analysis::fundamental_frequency_with_confidence(
        pitch_slice,
        sample_rate,
        search_min,
        search_max,
    );
    let pitch_error_cents = if fundamental_hz > 0.0 && expected_fundamental_hz > 0.0 {
        expected_fundamental
            .cents_between(synth_core::types::Hertz::new(fundamental_hz))
            .as_f32()
    } else {
        0.0
    };

    // Per-channel fundamentals. For stereo input we re-run pitch detection
    // on the left and right channels independently using the SAME release/
    // sustain slice region (`attack_start..note_samples`) and the SAME
    // anchored search band as `fundamental_hz`. This lets the caller spot
    // wide-stereo patches where L and R carry different fundamentals — the
    // pooled `fundamental_hz` (computed on max(|L|,|R|)) reports a single
    // value that mixes both. For mono input both fields are `None` and the
    // analysis_signal_mode reflects that.
    let (
        analysis_signal_mode,
        fundamental_left,
        fundamental_right,
        fundamental_left_confidence,
        fundamental_right_confidence,
    ) = if rendered.channels >= 2 {
        let left_slice = left_samples
            .get(attack_start..note_samples.min(left_samples.len()))
            .unwrap_or(&left_samples);
        let right_slice = right_samples
            .get(attack_start..note_samples.min(right_samples.len()))
            .unwrap_or(&right_samples);
        let (f_l, c_l) = analysis::fundamental_frequency_with_confidence(
            left_slice,
            sample_rate,
            search_min,
            search_max,
        );
        let (f_r, c_r) = analysis::fundamental_frequency_with_confidence(
            right_slice,
            sample_rate,
            search_min,
            search_max,
        );
        (
            synth_mcp::types::AnalysisSignalMode::MaxAbsStereo,
            Some(f_l),
            Some(f_r),
            Some(c_l),
            Some(c_r),
        )
    } else {
        (
            synth_mcp::types::AnalysisSignalMode::Mono,
            None,
            None,
            None,
            None,
        )
    };

    let envelope_window_ms = 50.0;
    let rms_envelope = analysis::rms_envelope(&mono, sample_rate, envelope_window_ms);
    // Centroid envelope tracks brightness motion; use the phase-robust
    // signal so anti-phase content still produces a meaningful spectrum.
    let mut centroid_envelope =
        analysis::centroid_envelope(&analysis_signal, sample_rate, envelope_window_ms);
    let rms_overall = analysis::rms_overall(&mono);

    // Trim only the centroid envelope tail. The raw `rms_envelope` is left
    // alone so `envelope_estimate` (which infers release length from RMS
    // decay) can see the full tail. Threshold = 5 % of overall RMS or 1e-4,
    // whichever is higher; never trim below 4 windows. Report how many were
    // trimmed so the agent can interpret a short centroid_envelope.
    let noise_floor = (rms_overall * 0.05).max(1e-4);
    let mut trimmed_tail_windows: u32 = 0;
    while centroid_envelope.len() > 4 {
        let last_idx = centroid_envelope.len() - 1;
        let last_rms = rms_envelope.get(last_idx).copied().unwrap_or(0.0);
        if last_rms < noise_floor {
            centroid_envelope.pop();
            trimmed_tail_windows += 1;
        } else {
            break;
        }
    }

    // Per-window pitch envelope. Uses a longer window than the rms/centroid
    // envelopes because FFT bin resolution scales with window length: a
    // 50 ms window at 44.1 kHz puts only 2-3 bins inside a one-tritone
    // search band at C2 (~65 Hz), so spectral leakage from neighboring
    // harmonics can flip the winning bin and produce false "drift". A
    // 200 ms window quadruples the resolution (~5 Hz/bin) and tracks bass
    // fundamentals stably. Same anchored search band as `fundamental_hz`.
    let pitch_envelope_window_ms: f32 = 200.0;
    let pitch_envelope = analysis::pitch_envelope(
        &analysis_signal,
        sample_rate,
        pitch_envelope_window_ms,
        search_min,
        search_max,
        1.0e-3,
    );

    // Stereo correlation needs the original interleaved buffer; only meaningful
    // when there are at least two channels, otherwise mono is "perfectly
    // correlated with itself" → 1.0.
    let stereo_correlation = if rendered.channels >= 2 {
        analysis::stereo_correlation(&rendered.samples)
    } else {
        1.0
    };

    let energy_bands: synth_mcp::types::AnalyzeEnergyBands =
        analysis::energy_bands(&analysis_signal, sample_rate).into();
    let harmonic_content: synth_mcp::types::AnalyzeHarmonicContent =
        analysis::harmonic_content(&analysis_signal, sample_rate, fundamental_hz).into();
    let envelope_estimate: synth_mcp::types::AnalyzeEnvelopeEstimate =
        analysis::envelope_estimate(&rms_envelope, envelope_window_ms, duration_ms).into();

    // Centroid trend over the held portion only. Slice `centroid_envelope` to
    // the note-on duration so the release tail doesn't bias the regression.
    let held_windows = ((f64::from(duration_ms) / f64::from(envelope_window_ms)).floor()) as usize;
    let centroid_trend_slice = if held_windows > 0 && held_windows <= centroid_envelope.len() {
        &centroid_envelope[..held_windows]
    } else {
        &centroid_envelope[..]
    };
    let centroid_trend_hz_per_sec =
        analysis::centroid_trend(centroid_trend_slice, envelope_window_ms);

    let peak_amplitude = analysis::peak_amplitude(&mono);
    let dc_offset = analysis::dc_offset(&mono);
    let clipped_samples = analysis::count_clipped(&mono, 0.999);

    // Per-channel and mid/side metrics (only when stereo).
    let (peak_left, peak_right, rms_left, rms_right, dc_left, dc_right, clipped_l, clipped_r) =
        if rendered.channels >= 2 {
            (
                Some(analysis::peak_amplitude(&left_samples)),
                Some(analysis::peak_amplitude(&right_samples)),
                Some(analysis::rms_overall(&left_samples)),
                Some(analysis::rms_overall(&right_samples)),
                Some(analysis::dc_offset(&left_samples)),
                Some(analysis::dc_offset(&right_samples)),
                Some(analysis::count_clipped(&left_samples, 0.999)),
                Some(analysis::count_clipped(&right_samples, 0.999)),
            )
        } else {
            (None, None, None, None, None, None, None, None)
        };
    // Bug 3 fix: stereo_width is a continuous 0..1 measure
    // `side_rms / (mid_rms + side_rms)`. 0 = mono (energy in mid only),
    // ~0.5 = typical stereo, 1 = anti-phase / fully decorrelated (energy
    // in side only). The earlier `s / m` form returned 0 for anti-phase
    // signals — semantically "mono", the OPPOSITE of what they are.
    let (mid_rms, side_rms, stereo_width) = if rendered.channels >= 2 {
        let m = analysis::rms_overall(&mid_samples);
        let s = analysis::rms_overall(&side_samples);
        let denom = m + s;
        let w = if denom > 1.0e-9 { s / denom } else { 0.0 };
        (Some(m), Some(s), Some(w))
    } else {
        (None, None, None)
    };

    // Stereo-aware flags: clipping if EITHER channel clipped, DC if EITHER
    // channel exceeds threshold, silent only if BOTH channels are silent.
    // Per-channel data takes precedence over the mono mix when present.
    let stereo_clipping = clipped_l.unwrap_or(0) > 0 || clipped_r.unwrap_or(0) > 0;
    let stereo_dc = dc_left.unwrap_or(0.0).abs() > 0.01 || dc_right.unwrap_or(0.0).abs() > 0.01;
    let stereo_silent = match (peak_left, peak_right) {
        (Some(l), Some(r)) => l < 0.005 && r < 0.005,
        _ => peak_amplitude < 0.005,
    };
    let stereo_low_output = match (peak_left, peak_right) {
        (Some(l), Some(r)) => l.max(r) < 0.05,
        _ => peak_amplitude < 0.05,
    };

    // off_pitch only fires when we are confident in the detected fundamental.
    // A low confidence (e.g. 0.2) means the loudest in-range bin is barely
    // taller than competing peaks — typical for filter-resonance latching
    // (Screamer Lead) or Karplus delay-line octave doubling.
    const PITCH_CONFIDENCE_THRESHOLD: f32 = 0.3;
    let off_pitch_real =
        pitch_error_cents.abs() > 50.0 && pitch_confidence >= PITCH_CONFIDENCE_THRESHOLD;

    let flags = synth_mcp::types::AnalyzeFlags {
        silent: stereo_silent,
        clipping: clipped_samples > 0 || stereo_clipping,
        has_dc_offset: dc_offset.abs() > 0.01 || stereo_dc,
        low_output: stereo_low_output,
        off_pitch: off_pitch_real,
    };

    synth_mcp::types::AnalyzeNoteResult {
        note_requested: note,
        note_played: rendered.effective_note.as_u8(),
        velocity,
        sample_rate,
        duration_seconds: rendered.duration_seconds,
        fundamental_hz,
        analysis_signal_mode,
        fundamental_left,
        fundamental_right,
        fundamental_left_confidence,
        fundamental_right_confidence,
        expected_fundamental_hz,
        pitch_error_cents,
        peak_amplitude,
        rms_overall,
        dc_offset,
        clipped_samples,
        envelope_window_ms,
        rms_envelope,
        centroid_envelope,
        spectrum_attack: to_peaks(analysis::spectrum_top_peaks(attack_slice, sample_rate, 8)),
        spectrum_sustain: to_peaks(analysis::spectrum_top_peaks(sustain_slice, sample_rate, 8)),
        spectrum_release: to_peaks(analysis::spectrum_top_peaks(release_slice, sample_rate, 8)),
        pitch_envelope,
        pitch_envelope_window_ms,
        stereo_correlation,
        energy_bands,
        harmonic_content,
        envelope_estimate,
        centroid_trend_hz_per_sec,
        flags,
        peak_left,
        peak_right,
        rms_left,
        rms_right,
        dc_left,
        dc_right,
        clipped_left: clipped_l,
        clipped_right: clipped_r,
        mid_rms,
        side_rms,
        stereo_width,
        pitch_confidence: Some(pitch_confidence),
        trimmed_tail_windows: if trimmed_tail_windows > 0 {
            Some(trimmed_tail_windows)
        } else {
            None
        },
        attack_window_start_ms: Some(attack_window_start_ms),
        sustain_window_start_ms: Some(sustain_window_start_ms),
        release_window_start_ms: Some(release_window_start_ms),
        warnings: rendered.warnings.clone(),
    }
}

impl From<crate::audio::analysis::SpectrumPeak> for synth_mcp::types::AnalyzeSpectrumPeak {
    fn from(p: crate::audio::analysis::SpectrumPeak) -> Self {
        Self {
            freq_hz: p.freq_hz,
            magnitude_db: p.magnitude_db,
        }
    }
}

impl From<crate::audio::analysis::EnergyBands> for synth_mcp::types::AnalyzeEnergyBands {
    fn from(b: crate::audio::analysis::EnergyBands) -> Self {
        Self {
            sub: b.sub,
            low: b.low,
            mid: b.mid,
            high: b.high,
        }
    }
}

impl From<crate::audio::analysis::HarmonicContent> for synth_mcp::types::AnalyzeHarmonicContent {
    fn from(h: crate::audio::analysis::HarmonicContent) -> Self {
        Self {
            thd_db: h.thd_db,
            odd_even_ratio_db: h.odd_even_ratio_db,
            n_harmonics: h.n_harmonics,
        }
    }
}

impl From<crate::audio::analysis::EnvelopeEstimate> for synth_mcp::types::AnalyzeEnvelopeEstimate {
    fn from(e: crate::audio::analysis::EnvelopeEstimate) -> Self {
        Self {
            attack_ms: e.attack_ms,
            decay_ms: e.decay_ms,
            sustain_level: e.sustain_level,
            release_ms: e.release_ms,
        }
    }
}

/// Format a parameter value for human-readable display.
///
/// When `unit` is provided, defers to `ParameterUnit::format` so the declared
/// unit on the parameter descriptor is honored. When the unit is `None` or
/// `ParameterUnit::None`, falls back to a name-based heuristic that picks a
/// reasonable suffix from the parameter's name. The heuristic should only be
/// reached when the descriptor is unavailable; prefer to pass the unit.
fn format_param_display(param: &Param, unit: Option<ParameterUnit>) -> String {
    let value = param.as_f32();

    if let Some(u) = unit
        && u != ParameterUnit::None
    {
        return u.format(value);
    }

    let name_lower = param.name().to_ascii_lowercase();
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

/// Build a [`ModuleTypeInfo`] from a [`ModuleType`] and its descriptor.
fn build_module_type_info(
    mt: synth_core::ModuleType,
    desc: &synth_core::ModuleDescriptor,
) -> ModuleTypeInfo {
    use synth_core::PortDirection;

    let category = if mt.is_voice_module() {
        "voice"
    } else if mt.is_effect() {
        "effect"
    } else {
        "visualizer"
    };

    let port_to_info = |p: &synth_core::PortDescriptor| synth_mcp::types::PortTypeInfo {
        name: p.name.to_string(),
        signal_type: port_type_str(p.port_type).to_owned(),
    };

    let input_ports = desc
        .ports
        .iter()
        .filter(|p| p.direction == PortDirection::Input)
        .map(port_to_info)
        .collect();
    let output_ports = desc
        .ports
        .iter()
        .filter(|p| p.direction == PortDirection::Output)
        .map(port_to_info)
        .collect();
    let parameters = desc
        .parameters
        .iter()
        .map(|p| ParamTypeInfo {
            name: p.name.clone(),
            description: p.description.clone(),
            min: p.range.min,
            max: p.range.max,
            default: p.range.default,
            unit: p.unit.suffix().to_owned(),
            choices: p.choices.as_ref().map(|opts| {
                opts.iter()
                    .enumerate()
                    .map(|(i, c)| synth_mcp::types::ChoiceInfo {
                        value: i as f32,
                        id: c.id.clone(),
                        name: c.name.clone(),
                    })
                    .collect()
            }),
        })
        .collect();

    ModuleTypeInfo {
        type_key: mt.prefix().to_string(),
        name: mt.name().to_string(),
        description: desc.description.clone(),
        category: category.to_string(),
        input_ports,
        output_ports,
        parameters,
        signal_flow_hint: signal_flow_hint(&desc.category),
    }
}

/// Convert a `PortType` to its string name.
fn port_type_str(pt: synth_core::PortType) -> &'static str {
    match pt {
        synth_core::PortType::Audio => "audio",
        synth_core::PortType::Control => "control",
        synth_core::PortType::Gate => "gate",
        synth_core::PortType::Midi => "midi",
    }
}

/// Return a hint about which types a given port type can connect to.
fn compatible_types_hint(pt: synth_core::PortType) -> &'static str {
    match pt {
        synth_core::PortType::Audio => "audio, control",
        synth_core::PortType::Control => "audio, control",
        synth_core::PortType::Gate => "gate, control",
        synth_core::PortType::Midi => "midi",
    }
}

/// Return a signal flow hint based on module category.
fn signal_flow_hint(category: &synth_core::ModuleCategory) -> Option<String> {
    use synth_core::ModuleCategory;
    match category {
        ModuleCategory::Oscillator => Some(
            "Connect 'out' → filter or mixer input. Use 'gate' and 'freq' CV inputs from note data."
                .to_owned(),
        ),
        ModuleCategory::Filter => Some(
            "Connect audio 'in' from oscillator/mixer, 'out' → amplifier. Use 'cutoff_cv' for envelope modulation."
                .to_owned(),
        ),
        ModuleCategory::Amplifier => Some(
            "Connect audio 'in' from filter, 'out' → output module. Use 'cv_gain' from envelope for volume shaping."
                .to_owned(),
        ),
        ModuleCategory::Envelope => Some(
            "Connect 'out' → amplifier 'cv_gain' or filter 'cutoff_cv'. Needs 'gate' input from note data."
                .to_owned(),
        ),
        ModuleCategory::LFO => Some(
            "Connect 'out' → any CV input for modulation (e.g. filter cutoff, oscillator frequency)."
                .to_owned(),
        ),
        ModuleCategory::Mixer => Some(
            "Connect multiple audio sources to 'in1'..'in8', output mixed signal from 'out'."
                .to_owned(),
        ),
        ModuleCategory::Output => Some(
            "Final module in voice chain. Connect audio to 'in_l'/'in_r'. Sends audio to instrument output."
                .to_owned(),
        ),
        ModuleCategory::Effect => Some(
            "Effect module in the instrument's effect chain. Audio passes through automatically."
                .to_owned(),
        ),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Group B — symbolic composition helper bridge impls
// ---------------------------------------------------------------------------

pub fn generate_chord_impl(
    symbol: &str,
    octave: i32,
    voicing: Option<&str>,
) -> Result<synth_mcp::types::GenerateChordResult, McpBridgeError> {
    use crate::composition::{ChordVoicing, generate_chord};
    use synth_mcp::types::GenerateChordResult;

    let v = match voicing {
        None => ChordVoicing::Close,
        Some(s) => ChordVoicing::from_str_opt(s).ok_or_else(|| {
            McpBridgeError::Other(format!(
                "unknown voicing {s:?}; expected one of close, drop2, drop3, open"
            ))
        })?,
    };
    let generated =
        generate_chord(symbol, octave, v).map_err(|e| McpBridgeError::Other(e.to_string()))?;
    Ok(GenerateChordResult {
        symbol: symbol.to_string(),
        root_pitch_class: generated.root_pitch_class,
        quality: generated.quality.to_string(),
        suffix: generated.suffix.to_string(),
        voicing: generated.voicing.as_str().to_string(),
        notes: generated.notes,
        warnings: generated.warnings,
    })
}

pub fn transpose_notes_impl(
    shared: &McpSharedState,
    pattern_id: u32,
    semitones: i32,
    scale_tonic: Option<u8>,
    scale_name: Option<&str>,
    tie_break: Option<&str>,
) -> Result<synth_mcp::types::TransposeNotesResult, McpBridgeError> {
    use crate::composition::{ScaleConstraint, transpose_pitches};
    use synth_mcp::types::TransposeNotesResult;
    use synth_sequencer::PatternId;

    let tie_break = parse_tie_break(tie_break)?;
    let scale = match (scale_tonic, scale_name) {
        (Some(t), Some(n)) => Some(ScaleConstraint::new(t, n)),
        _ => None,
    };
    let mut warnings = Vec::new();
    if scale_tonic.is_some() ^ scale_name.is_some() {
        warnings.push(
            "scale_tonic and scale_name must both be set to enable scale snapping; ignoring the partial constraint".to_string(),
        );
    }

    let mut song = shared.song.write();
    let pattern = song
        .pattern_mut(PatternId(pattern_id))
        .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

    // Snapshot pitches, transpose in place, write back via `note_mut` so note
    // IDs / durations / instruments survive.
    let (ids, mut pitches): (Vec<_>, Vec<u8>) = pattern
        .notes()
        .iter()
        .map(|n| (n.id, n.pitch.as_midi()))
        .unzip();
    let result = transpose_pitches(&mut pitches, semitones, scale.as_ref(), tie_break);
    write_back_pitches(pattern, &ids, &pitches);

    Ok(TransposeNotesResult {
        pattern_id,
        semitones,
        notes_in: result.notes_in,
        notes_transposed: result.notes_transposed,
        notes_out_of_range: result.notes_out_of_range,
        notes_snapped_to_scale: result.notes_snapped_to_scale,
        scale_tonic_pitch_class: scale.as_ref().map(|s| s.tonic),
        scale_name: scale.as_ref().map(|s| s.scale_name.to_string()),
        warnings,
    })
}

pub fn quantize_notes_to_scale_impl(
    shared: &McpSharedState,
    pattern_id: u32,
    scale_tonic: u8,
    scale_name: &str,
    tie_break: Option<&str>,
) -> Result<synth_mcp::types::QuantizeNotesToScaleResult, McpBridgeError> {
    use crate::composition::{ScaleConstraint, ScaleQuantizeOptions, quantize_pitches_to_scale};
    use synth_mcp::types::QuantizeNotesToScaleResult;
    use synth_sequencer::PatternId;

    let tie_break = parse_tie_break(tie_break)?;
    let scale = ScaleConstraint::new(scale_tonic, scale_name);
    let scale_label = scale.scale_name.to_string();
    let scale_pc = scale.tonic;

    let mut song = shared.song.write();
    let pattern = song
        .pattern_mut(PatternId(pattern_id))
        .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

    let (ids, mut pitches): (Vec<_>, Vec<u8>) = pattern
        .notes()
        .iter()
        .map(|n| (n.id, n.pitch.as_midi()))
        .unzip();
    let result = quantize_pitches_to_scale(
        &mut pitches,
        &ScaleQuantizeOptions {
            scale: &scale,
            tie_break,
        },
    );
    write_back_pitches(pattern, &ids, &pitches);

    let mean_correction = if result.notes_moved > 0 {
        result.total_correction_semitones as f32 / result.notes_moved as f32
    } else {
        0.0
    };

    Ok(QuantizeNotesToScaleResult {
        pattern_id,
        scale_tonic_pitch_class: scale_pc,
        scale_name: scale_label,
        notes_in: result.notes_in,
        notes_already_in_scale: result.notes_already_in_scale,
        notes_moved: result.notes_moved,
        mean_correction_semitones: mean_correction,
        max_correction_semitones: result.max_correction_semitones,
        warnings: Vec::new(),
    })
}

pub fn quantize_notes_to_grid_impl(
    shared: &McpSharedState,
    pattern_id: u32,
    grid_ticks: u32,
    strength: Option<f32>,
    swing: Option<f32>,
    humanize_ticks: Option<u32>,
    humanize_seed: Option<u64>,
) -> Result<synth_mcp::types::QuantizeNotesToGridResult, McpBridgeError> {
    use crate::composition::{GridQuantizeOptions, NoteTiming, quantize_grid};
    use synth_mcp::types::QuantizeNotesToGridResult;
    use synth_sequencer::PatternId;

    let strength_val = strength.unwrap_or(1.0).clamp(0.0, 1.0);
    let swing_val = swing.unwrap_or(0.0).clamp(0.0, 1.0);
    let humanize_val = humanize_ticks.unwrap_or(0);
    let seed_val = humanize_seed.unwrap_or(0);

    let mut song = shared.song.write();
    let pattern = song
        .pattern_mut(PatternId(pattern_id))
        .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

    let length_ticks = pattern.length.0;
    let (ids, mut timings): (Vec<_>, Vec<NoteTiming>) = pattern
        .notes()
        .iter()
        .map(|n| {
            (
                n.id,
                NoteTiming {
                    start_tick: n.start.0,
                },
            )
        })
        .unzip();
    let result = quantize_grid(
        &mut timings,
        &GridQuantizeOptions {
            grid_ticks,
            pattern_length_ticks: length_ticks,
            strength: strength_val,
            swing: swing_val,
            humanize_ticks: humanize_val,
            seed: seed_val,
        },
    );

    // Goes through `move_note` so the pattern's start-tick sort invariant is
    // preserved — writing `note_mut().start = ...` would silently break it.
    for (id, timing) in ids.iter().zip(timings.iter()) {
        pattern.move_note(*id, synth_sequencer::PatternTick(timing.start_tick));
    }

    let mean_delta = if result.notes_moved > 0 {
        result.total_delta_ticks as f32 / result.notes_moved as f32
    } else {
        0.0
    };

    let mut warnings = Vec::new();
    if result.disabled {
        warnings.push("grid_ticks was 0; no changes applied".to_string());
    }

    Ok(QuantizeNotesToGridResult {
        pattern_id,
        grid_ticks,
        strength: strength_val,
        swing: swing_val,
        humanize_ticks: humanize_val,
        humanize_seed: seed_val,
        notes_in: result.notes_in,
        notes_moved: result.notes_moved,
        mean_delta_ticks: mean_delta,
        max_delta_ticks: result.max_delta_ticks,
        pattern_length_ticks: length_ticks,
        warnings,
    })
}

fn write_back_pitches(
    pattern: &mut synth_sequencer::Pattern,
    ids: &[synth_sequencer::NoteId],
    pitches: &[u8],
) {
    for (id, new_pitch) in ids.iter().zip(pitches.iter()) {
        if let Some(note) = pattern.note_mut(*id)
            && let Some(p) = synth_sequencer::Pitch::new(*new_pitch)
        {
            note.pitch = p;
        }
    }
}

fn parse_tie_break(s: Option<&str>) -> Result<crate::composition::ScaleTieBreak, McpBridgeError> {
    use crate::composition::ScaleTieBreak;
    match s {
        None => Ok(ScaleTieBreak::NearestUp),
        Some(raw) => ScaleTieBreak::from_str_opt(raw).ok_or_else(|| {
            McpBridgeError::Other(format!(
                "unknown tie_break {raw:?}; expected one of up, down, nearest"
            ))
        }),
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod pre_master_peak_tests {
    use super::pre_master_peak_for;
    use synth_core::{BipolarValue, Gain};

    /// Default rig: pan = center, volume = MAX. Per-channel peaks both equal
    /// `internal × 0.7071` (constant-power pan-law at center). Dividing each
    /// channel peak by its own gain restores the internal value, and we take
    /// the larger of the two — should land at the internal patch peak.
    #[test]
    fn reverses_constant_power_pan_law_at_center() {
        let internal = 0.8_f32;
        let attenuated = internal * std::f32::consts::FRAC_1_SQRT_2;
        let gains = (Gain::new(1.0), BipolarValue::CENTER);
        let (peak, dbfs) = pre_master_peak_for(Some(&gains), attenuated, attenuated);
        assert!((peak - internal).abs() < 1e-4, "got {peak}");
        assert!((dbfs - 20.0 * internal.log10()).abs() < 1e-3);
    }

    /// Volume drop should also be reversed: at half volume the rendered peak
    /// halves on top of the pan-law attenuation; the restored peak must still
    /// match the internal value.
    #[test]
    fn reverses_volume_drop() {
        let internal = 0.8_f32;
        let rendered = internal * 0.5 * std::f32::consts::FRAC_1_SQRT_2;
        let gains = (Gain::new(0.5), BipolarValue::CENTER);
        let (peak, _) = pre_master_peak_for(Some(&gains), rendered, rendered);
        assert!((peak - internal).abs() < 1e-4, "got {peak}");
    }

    /// Hard-panned signals leave one channel silent. The silent channel's
    /// `peak / 0` division must be skipped (1e-6 floor) and the live channel
    /// should drive the result.
    #[test]
    fn handles_hard_pan_without_division_by_zero() {
        let internal = 0.6_f32;
        let gains = (Gain::new(1.0), BipolarValue::new(1.0)); // full right
        // Only the right channel carries signal; left is silent.
        let (peak, _) = pre_master_peak_for(Some(&gains), 0.0, internal);
        assert!((peak - internal).abs() < 1e-4, "got {peak}");
    }

    /// Missing snapshot (instrument id we don't know) falls back to the raw
    /// per-channel peak so the caller still gets a meaningful lower bound.
    #[test]
    fn falls_back_to_raw_peak_when_gains_unknown() {
        let (peak, _) = pre_master_peak_for(None, 0.3, 0.6);
        assert!((peak - 0.6).abs() < 1e-4, "got {peak}");
    }

    /// Silence in → silence out, dBFS clamps to the silent floor instead of
    /// reporting `-inf`.
    #[test]
    fn silence_reports_silent_floor() {
        let gains = (Gain::new(1.0), BipolarValue::CENTER);
        let (peak, dbfs) = pre_master_peak_for(Some(&gains), 0.0, 0.0);
        assert_eq!(peak, 0.0);
        assert_eq!(dbfs, crate::audio::mix_analysis::SILENT_FLOOR_DBFS);
    }
}
