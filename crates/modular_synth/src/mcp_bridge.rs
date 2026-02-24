//! Bridge between the MCP server and the running synth engine.
//!
//! `AppSynthBridge` implements `SynthBridge` by reading from `EngineState`
//! (shared graph, meters, transport) and sending commands via `CommandSender`.

use std::sync::Arc;

use synth_core::{MidiNote, Param, Velocity};
use synth_engine::instrument::{InstrumentId, MidiChannel};
use synth_engine::state::EngineState;
use synth_engine::{CommandSender, EngineCommand};
use synth_mcp::bridge::SynthBridge;
use synth_mcp::error::McpBridgeError;
use synth_mcp::types::{
    ConnectionInfo, DiagnosticSeverity, EngineStatus, GraphDiagnostic, InstrumentInfo, ModuleInfo,
    ParameterInfo,
};

/// Bridge implementation for the modular_synth application.
pub struct AppSynthBridge {
    state: Arc<EngineState>,
    command_sender: CommandSender,
}

impl AppSynthBridge {
    /// Create a new bridge with access to the engine state and command sender.
    pub fn new(state: Arc<EngineState>, command_sender: CommandSender) -> Self {
        Self {
            state,
            command_sender,
        }
    }
}

impl SynthBridge for AppSynthBridge {
    fn list_instruments(&self) -> Result<Vec<InstrumentInfo>, McpBridgeError> {
        // We expose the default instrument (ID 0).
        // Full instrument tracking would require additional shared state.
        Ok(vec![InstrumentInfo {
            id: 0,
            name: "Default".to_string(),
            midi_channel: 1,
            enabled: true,
            module_count: self.state.shared_graph.module_count(),
            effect_count: 0,
        }])
    }

    fn get_instrument_info(&self, instrument_id: u64) -> Result<InstrumentInfo, McpBridgeError> {
        if instrument_id != 0 {
            return Err(McpBridgeError::InstrumentNotFound(instrument_id));
        }
        Ok(InstrumentInfo {
            id: 0,
            name: "Default".to_string(),
            midi_channel: 1,
            enabled: true,
            module_count: self.state.shared_graph.module_count(),
            effect_count: 0,
        })
    }

    fn list_modules(&self, instrument_id: u64) -> Result<Vec<ModuleInfo>, McpBridgeError> {
        if instrument_id != 0 {
            return Err(McpBridgeError::InstrumentNotFound(instrument_id));
        }

        let modules = self.state.shared_graph.get_all_modules();
        let connections = self.state.shared_graph.get_connections();

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
        if instrument_id != 0 {
            return Err(McpBridgeError::InstrumentNotFound(instrument_id));
        }

        let connections = self.state.shared_graph.get_connections();
        Ok(connections
            .into_iter()
            .map(|c| ConnectionInfo {
                from_module: c.from_module.to_string(),
                from_port: c.from_port,
                to_module: c.to_module.to_string(),
                to_port: c.to_port,
                signal_level: c.signal_level,
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
        let (peak_left, peak_right) = self.state.meters.get_peak();
        let (rms_left, rms_right) = self.state.meters.get_rms();

        Ok(EngineStatus {
            cpu_usage: self.state.cpu_usage.load(),
            voice_count: self.state.voice_count.load(),
            sample_rate: self.state.sample_rate.load(),
            peak_left,
            peak_right,
            rms_left,
            rms_right,
            master_volume: self.state.master_volume.load(),
            tempo: self.state.transport.get_tempo(),
            is_playing: self.state.transport.is_playing(),
            instrument_count: 1,
        })
    }

    fn get_graph_diagnostics(
        &self,
        instrument_id: u64,
    ) -> Result<Vec<GraphDiagnostic>, McpBridgeError> {
        if instrument_id != 0 {
            return Err(McpBridgeError::InstrumentNotFound(instrument_id));
        }

        let mut diagnostics = Vec::new();
        let modules = self.state.shared_graph.get_all_modules();
        let connections = self.state.shared_graph.get_connections();

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

            if !has_input && !has_output {
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

    fn set_parameter(
        &self,
        instrument_id: u64,
        module_id: &str,
        param_name: &str,
        value: f32,
    ) -> Result<(), McpBridgeError> {
        if instrument_id != 0 {
            return Err(McpBridgeError::InstrumentNotFound(instrument_id));
        }

        // Find the module and its current parameter to construct the correct Param variant
        let module_snapshot = self
            .state
            .shared_graph
            .get_all_modules()
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

        if self.command_sender.send(EngineCommand::SetModuleParameter {
            instrument_id: Some(InstrumentId::new(instrument_id)),
            module_id: module_snapshot.id,
            param: new_param,
        }) {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed)
        }
    }

    fn note_on(&self, note: u8, velocity: u8, channel: u8) -> Result<(), McpBridgeError> {
        let midi_channel = MidiChannel::from_one_indexed(channel).unwrap_or(MidiChannel::CH1);
        if self.command_sender.send(EngineCommand::NoteOn {
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
        if self.command_sender.send(EngineCommand::NoteOff {
            note: MidiNote::new(note),
            channel: midi_channel,
        }) {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed)
        }
    }
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
