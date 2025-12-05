//! Module connectivity and status types.
//!
//! This module provides types for tracking module connectivity status,
//! voice stealing reasons, and module errors for GUI visualization.

use serde::{Deserialize, Serialize};

use super::commands::ModuleId;
use super::typed_params::Param;

/// Module connectivity status for UI visualization.
///
/// This indicates whether a module is contributing to the audio output,
/// allowing the GUI to dim or highlight modules accordingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ModuleConnectivityStatus {
    /// Module has no connections (isolated).
    /// GUI should show as dimmed/grayed out.
    #[default]
    Disconnected,
    /// Module has some connections but is not in the signal path to output.
    /// GUI should show as partially visible.
    PartiallyConnected,
    /// Module is fully connected and contributing to output.
    /// GUI should show at full brightness with activity indication.
    Connected,
    /// Module is connected but bypassed.
    /// GUI should show as connected but with bypass indication.
    Bypassed,
}

impl ModuleConnectivityStatus {
    /// Get the opacity value for rendering (0.0 - 1.0).
    pub fn opacity(&self) -> f32 {
        match self {
            ModuleConnectivityStatus::Disconnected => 0.4,
            ModuleConnectivityStatus::PartiallyConnected => 0.7,
            ModuleConnectivityStatus::Connected => 1.0,
            ModuleConnectivityStatus::Bypassed => 0.5,
        }
    }

    /// Check if the module is "live" (contributing to output).
    pub fn is_live(&self) -> bool {
        matches!(self, ModuleConnectivityStatus::Connected)
    }

    /// Check if the module has any connections.
    pub fn has_connections(&self) -> bool {
        !matches!(self, ModuleConnectivityStatus::Disconnected)
    }

    /// Get a description of this status.
    pub fn description(&self) -> &'static str {
        match self {
            ModuleConnectivityStatus::Disconnected => "Not connected",
            ModuleConnectivityStatus::PartiallyConnected => {
                "Partially connected (not in signal path)"
            }
            ModuleConnectivityStatus::Connected => "Connected and active",
            ModuleConnectivityStatus::Bypassed => "Bypassed",
        }
    }
}

/// Reasons for voice stealing.
/// Useful for UI feedback to show users when and why voices were stolen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VoiceStealReason {
    /// Maximum polyphony was reached, oldest/quietest voice was stolen.
    MaxPolyphonyReached,
    /// Same note was triggered again (retrigger).
    SameNoteRetrigger,
    /// Voice was stolen based on priority settings.
    PriorityBased,
    /// Voice was stolen because it was in release phase.
    ReleasePhaseSteal,
    /// Voice was stolen because it was the quietest.
    QuietestVoiceSteal,
}

impl VoiceStealReason {
    /// Get a human-readable description.
    pub fn description(&self) -> &'static str {
        match self {
            VoiceStealReason::MaxPolyphonyReached => "Maximum polyphony reached",
            VoiceStealReason::SameNoteRetrigger => "Note retriggered",
            VoiceStealReason::PriorityBased => "Lower priority voice replaced",
            VoiceStealReason::ReleasePhaseSteal => "Released voice replaced",
            VoiceStealReason::QuietestVoiceSteal => "Quietest voice replaced",
        }
    }
}

/// Module error kinds for diagnostic display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ModuleErrorKind {
    /// Parameter value was out of range.
    ParameterOutOfRange {
        param: Param,
        value: f32,
        min: f32,
        max: f32,
    },
    /// Module is overloading CPU.
    ProcessingOverload { cpu_percent: f32 },
    /// Invalid connection attempt.
    InvalidConnection { reason: String },
    /// Internal module error.
    InternalError(String),
    /// Module output is clipping.
    OutputClipping { peak_level: f32 },
    /// Module produced NaN or infinity values.
    InvalidOutput,
    /// Module initialization failed.
    InitializationFailed(String),
}

impl ModuleErrorKind {
    /// Check if this is a critical error that should stop processing.
    pub fn is_critical(&self) -> bool {
        matches!(
            self,
            ModuleErrorKind::InvalidOutput | ModuleErrorKind::InitializationFailed(_)
        )
    }

    /// Check if this is a warning (non-critical).
    pub fn is_warning(&self) -> bool {
        matches!(
            self,
            ModuleErrorKind::OutputClipping { .. }
                | ModuleErrorKind::ProcessingOverload { .. }
                | ModuleErrorKind::ParameterOutOfRange { .. }
        )
    }

    /// Get a short description for UI display.
    pub fn short_description(&self) -> String {
        match self {
            ModuleErrorKind::ParameterOutOfRange { param, .. } => {
                format!("Parameter out of range: {:?}", param)
            }
            ModuleErrorKind::ProcessingOverload { cpu_percent } => {
                format!("CPU overload: {:.1}%", cpu_percent)
            }
            ModuleErrorKind::InvalidConnection { reason } => {
                format!("Invalid connection: {}", reason)
            }
            ModuleErrorKind::InternalError(msg) => {
                format!("Error: {}", msg)
            }
            ModuleErrorKind::OutputClipping { peak_level } => {
                format!("Clipping: {:.1} dB", 20.0 * peak_level.log10())
            }
            ModuleErrorKind::InvalidOutput => "Invalid output (NaN/Inf)".to_string(),
            ModuleErrorKind::InitializationFailed(msg) => {
                format!("Init failed: {}", msg)
            }
        }
    }
}

/// A module error event with context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleError {
    /// The module that generated the error.
    pub module_id: ModuleId,
    /// The error kind.
    pub error: ModuleErrorKind,
    /// Sample position when error occurred.
    pub timestamp: u64,
    /// Number of times this error has occurred recently.
    pub occurrence_count: u32,
}

impl ModuleError {
    /// Create a new module error.
    pub fn new(module_id: ModuleId, error: ModuleErrorKind, timestamp: u64) -> Self {
        Self {
            module_id,
            error,
            timestamp,
            occurrence_count: 1,
        }
    }
}

/// Port visual state for GUI rendering.
#[derive(Debug, Clone, Default)]
pub struct PortVisualState {
    /// Whether this port is connected.
    pub connected: bool,
    /// Current signal level through this port (0.0 - 1.0+).
    pub signal_level: f32,
    /// Whether signal is actively flowing.
    pub is_active: bool,
    /// Number of connections to/from this port.
    pub connection_count: usize,
}

impl PortVisualState {
    /// Create a connected port state.
    pub fn connected(connection_count: usize) -> Self {
        Self {
            connected: true,
            signal_level: 0.0,
            is_active: false,
            connection_count,
        }
    }

    /// Update the signal level with smoothing.
    pub fn update_level(&mut self, new_level: f32, smoothing: f32) {
        self.signal_level = self.signal_level * smoothing + new_level * (1.0 - smoothing);
        self.is_active = new_level > 0.001;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connectivity_status() {
        assert_eq!(ModuleConnectivityStatus::Disconnected.opacity(), 0.4);
        assert_eq!(ModuleConnectivityStatus::Connected.opacity(), 1.0);
        assert!(ModuleConnectivityStatus::Connected.is_live());
        assert!(!ModuleConnectivityStatus::Disconnected.is_live());
    }

    #[test]
    fn test_voice_steal_reason() {
        let reason = VoiceStealReason::MaxPolyphonyReached;
        assert!(!reason.description().is_empty());
    }

    #[test]
    fn test_module_error_severity() {
        let warning = ModuleErrorKind::OutputClipping { peak_level: 1.5 };
        assert!(warning.is_warning());
        assert!(!warning.is_critical());

        let critical = ModuleErrorKind::InvalidOutput;
        assert!(critical.is_critical());
        assert!(!critical.is_warning());
    }

    #[test]
    fn test_port_visual_state() {
        let mut port = PortVisualState::connected(2);
        assert!(port.connected);
        assert_eq!(port.connection_count, 2);

        port.update_level(0.8, 0.5);
        assert!(port.is_active);
        assert!(port.signal_level > 0.0);
    }
}
