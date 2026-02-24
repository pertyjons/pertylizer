//! Serializable response types for MCP tools.

use serde::Serialize;

/// Information about an instrument.
#[derive(Debug, Clone, Serialize)]
pub struct InstrumentInfo {
    /// Instrument ID.
    pub id: u64,
    /// Instrument name.
    pub name: String,
    /// MIDI channel (1-16).
    pub midi_channel: u8,
    /// Whether the instrument is enabled.
    pub enabled: bool,
    /// Number of modules in the voice graph.
    pub module_count: usize,
    /// Number of effects in the chain.
    pub effect_count: usize,
}

/// Information about a module in the voice graph.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleInfo {
    /// Module ID string (e.g. "osc-1").
    pub id: String,
    /// Module type (e.g. "Oscillator").
    pub module_type: String,
    /// Human-readable name.
    pub name: String,
    /// Whether the module is bypassed.
    pub bypassed: bool,
    /// Current parameters.
    pub parameters: Vec<ParameterInfo>,
    /// Input port names.
    pub input_ports: Vec<String>,
    /// Output port names.
    pub output_ports: Vec<String>,
}

/// Information about a parameter.
#[derive(Debug, Clone, Serialize)]
pub struct ParameterInfo {
    /// Parameter name (e.g. "frequency").
    pub name: String,
    /// Current value as f32.
    pub value: f32,
    /// Human-readable display value (e.g. "440.0 Hz").
    pub display: String,
}

/// Information about a connection between modules.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionInfo {
    /// Source module ID.
    pub from_module: String,
    /// Source port name.
    pub from_port: String,
    /// Destination module ID.
    pub to_module: String,
    /// Destination port name.
    pub to_port: String,
    /// Current signal level (0.0-1.0).
    pub signal_level: f32,
}

/// Engine status information.
#[derive(Debug, Clone, Serialize)]
pub struct EngineStatus {
    /// CPU usage (0.0-1.0).
    pub cpu_usage: f32,
    /// Number of active voices.
    pub voice_count: u32,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Peak meters (left, right).
    pub peak_left: f32,
    pub peak_right: f32,
    /// RMS meters (left, right).
    pub rms_left: f32,
    pub rms_right: f32,
    /// Master volume (0.0-2.0).
    pub master_volume: f32,
    /// Transport tempo in BPM.
    pub tempo: f32,
    /// Whether transport is playing.
    pub is_playing: bool,
    /// Number of instruments.
    pub instrument_count: usize,
}

/// A diagnostic issue found in the graph.
#[derive(Debug, Clone, Serialize)]
pub struct GraphDiagnostic {
    /// Severity level.
    pub severity: DiagnosticSeverity,
    /// Module ID involved (if applicable).
    pub module_id: Option<String>,
    /// Description of the issue.
    pub message: String,
}

/// Severity of a diagnostic.
#[derive(Debug, Clone, Serialize)]
pub enum DiagnosticSeverity {
    /// Informational note.
    Info,
    /// Potential problem.
    Warning,
    /// Definite problem.
    Error,
}
