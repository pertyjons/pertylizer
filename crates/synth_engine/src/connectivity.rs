//! Module connectivity and status types.
//!
//! This module provides types for tracking module connectivity status,
//! voice stealing reasons, and module errors for GUI visualization.

use serde::{Deserialize, Serialize};

use super::commands::ModuleId;
use synth_core::{Amplitude, CpuUsage, NormalizedValue, Param};

// ============================================================================
// Port State Enums - Descriptive types instead of booleans
// ============================================================================

/// Connection state for a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionState {
    /// Port is not connected to anything.
    #[default]
    Disconnected,
    /// Port is connected to one or more cables.
    Connected,
}

impl ConnectionState {
    /// Check if the port is connected.
    #[inline]
    #[must_use]
    pub const fn is_connected(self) -> bool {
        matches!(self, Self::Connected)
    }
}

impl From<bool> for ConnectionState {
    fn from(connected: bool) -> Self {
        if connected {
            Self::Connected
        } else {
            Self::Disconnected
        }
    }
}

/// Signal activity state for a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignalActivity {
    /// No signal is flowing through the port.
    #[default]
    Inactive,
    /// Signal is actively flowing through the port.
    Active,
}

impl SignalActivity {
    /// Check if signal is active.
    #[inline]
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }
}

impl From<bool> for SignalActivity {
    fn from(active: bool) -> Self {
        if active { Self::Active } else { Self::Inactive }
    }
}

/// Count of connections to/from a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[must_use]
pub struct ConnectionCount(usize);

impl ConnectionCount {
    /// No connections.
    pub const ZERO: Self = Self(0);

    /// Create a new connection count.
    #[inline]
    pub const fn new(count: usize) -> Self {
        Self(count)
    }

    /// Get the raw count value.
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0
    }

    /// Check if there are any connections.
    #[inline]
    pub const fn has_connections(self) -> bool {
        self.0 > 0
    }

    /// Increment the count.
    #[inline]
    pub fn increment(&mut self) {
        self.0 += 1;
    }

    /// Decrement the count (saturating at zero).
    #[inline]
    pub fn decrement(&mut self) {
        self.0 = self.0.saturating_sub(1);
    }
}

impl From<usize> for ConnectionCount {
    fn from(count: usize) -> Self {
        Self(count)
    }
}

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
    #[must_use]
    pub fn opacity(&self) -> NormalizedValue {
        match self {
            Self::Disconnected => NormalizedValue::new(0.4),
            Self::PartiallyConnected => NormalizedValue::new(0.7),
            Self::Connected => NormalizedValue::MAX,
            Self::Bypassed => NormalizedValue::new(0.5),
        }
    }

    /// Check if the module is "live" (contributing to output).
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Connected)
    }

    /// Check if the module has any connections.
    pub fn has_connections(&self) -> bool {
        !matches!(self, Self::Disconnected)
    }

    /// Get a description of this status.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Disconnected => "Not connected",
            Self::PartiallyConnected => "Partially connected (not in signal path)",
            Self::Connected => "Connected and active",
            Self::Bypassed => "Bypassed",
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
            Self::MaxPolyphonyReached => "Maximum polyphony reached",
            Self::SameNoteRetrigger => "Note retriggered",
            Self::PriorityBased => "Lower priority voice replaced",
            Self::ReleasePhaseSteal => "Released voice replaced",
            Self::QuietestVoiceSteal => "Quietest voice replaced",
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
    ProcessingOverload { cpu_percent: CpuUsage },
    /// Invalid connection attempt.
    InvalidConnection { reason: String },
    /// Internal module error.
    InternalError(String),
    /// Module output is clipping.
    OutputClipping { peak_level: Amplitude },
    /// Module produced NaN or infinity values.
    InvalidOutput,
    /// Module initialization failed.
    InitializationFailed(String),
}

impl ModuleErrorKind {
    /// Check if this is a critical error that should stop processing.
    pub fn is_critical(&self) -> bool {
        matches!(self, Self::InvalidOutput | Self::InitializationFailed(_))
    }

    /// Check if this is a warning (non-critical).
    pub fn is_warning(&self) -> bool {
        matches!(
            self,
            Self::OutputClipping { .. }
                | Self::ProcessingOverload { .. }
                | Self::ParameterOutOfRange { .. }
        )
    }

    /// Get a short description for UI display.
    pub fn short_description(&self) -> String {
        match self {
            Self::ParameterOutOfRange { param, .. } => {
                format!("Parameter out of range: {:?}", param)
            }
            Self::ProcessingOverload { cpu_percent } => {
                format!("CPU overload: {:.1}%", cpu_percent.as_f32())
            }
            Self::InvalidConnection { reason } => {
                format!("Invalid connection: {}", reason)
            }
            Self::InternalError(msg) => {
                format!("Error: {}", msg)
            }
            Self::OutputClipping { peak_level } => {
                format!("Clipping: {:.1} dB", 20.0 * peak_level.as_f32().log10())
            }
            Self::InvalidOutput => "Invalid output (NaN/Inf)".to_string(),
            Self::InitializationFailed(msg) => {
                format!("Init failed: {}", msg)
            }
        }
    }
}

/// Sample timestamp for error tracking.
///
/// Represents a sample position but with serde support for serialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[must_use]
pub struct SampleTimestamp(u64);

impl SampleTimestamp {
    /// Create a new sample timestamp.
    #[inline]
    pub const fn new(samples: u64) -> Self {
        Self(samples)
    }

    /// Get the raw sample count.
    #[inline]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl From<u64> for SampleTimestamp {
    fn from(samples: u64) -> Self {
        Self(samples)
    }
}

/// Count of error occurrences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[must_use]
pub struct OccurrenceCount(u32);

impl OccurrenceCount {
    /// Single occurrence.
    pub const ONE: Self = Self(1);

    /// Create a new occurrence count.
    #[inline]
    pub const fn new(count: u32) -> Self {
        Self(count)
    }

    /// Get the raw count value.
    #[inline]
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Increment the count.
    #[inline]
    pub fn increment(&mut self) {
        self.0 = self.0.saturating_add(1);
    }
}

impl From<u32> for OccurrenceCount {
    fn from(count: u32) -> Self {
        Self(count)
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
    pub timestamp: SampleTimestamp,
    /// Number of times this error has occurred recently.
    pub occurrence_count: OccurrenceCount,
}

impl ModuleError {
    /// Create a new module error.
    pub fn new(module_id: ModuleId, error: ModuleErrorKind, timestamp: SampleTimestamp) -> Self {
        Self {
            module_id,
            error,
            timestamp,
            occurrence_count: OccurrenceCount::ONE,
        }
    }
}

/// Port visual state for GUI rendering.
#[derive(Debug, Clone, Default)]
pub struct PortVisualState {
    /// Whether this port is connected.
    pub connection: ConnectionState,
    /// Current signal level through this port.
    pub signal_level: Amplitude,
    /// Whether signal is actively flowing.
    pub activity: SignalActivity,
    /// Number of connections to/from this port.
    pub connection_count: ConnectionCount,
}

impl PortVisualState {
    /// Create a connected port state.
    #[must_use]
    pub fn connected(count: ConnectionCount) -> Self {
        Self {
            connection: ConnectionState::Connected,
            signal_level: Amplitude::ZERO,
            activity: SignalActivity::Inactive,
            connection_count: count,
        }
    }

    /// Check if the port is connected.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connection.is_connected()
    }

    /// Check if signal is active.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.activity.is_active()
    }

    /// Update the signal level with smoothing.
    pub fn update_level(&mut self, new_level: f32, smoothing: f32) {
        self.signal_level =
            Amplitude::new(self.signal_level.as_f32() * smoothing + new_level * (1.0 - smoothing));
        self.activity = SignalActivity::from(new_level > 0.001);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connectivity_status() {
        assert_eq!(
            ModuleConnectivityStatus::Disconnected.opacity(),
            NormalizedValue::new(0.4)
        );
        assert_eq!(
            ModuleConnectivityStatus::Connected.opacity(),
            NormalizedValue::MAX
        );
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
        let warning = ModuleErrorKind::OutputClipping {
            peak_level: synth_core::Amplitude::new(1.5),
        };
        assert!(warning.is_warning());
        assert!(!warning.is_critical());

        let critical = ModuleErrorKind::InvalidOutput;
        assert!(critical.is_critical());
        assert!(!critical.is_warning());
    }

    #[test]
    fn test_port_visual_state() {
        let mut port = PortVisualState::connected(ConnectionCount::new(2));
        assert!(port.is_connected());
        assert_eq!(port.connection_count.as_usize(), 2);

        port.update_level(0.8, 0.5);
        assert!(port.is_active());
        assert!(port.signal_level > synth_core::Amplitude::ZERO);
    }

    #[test]
    fn test_connection_count() {
        let mut count = ConnectionCount::ZERO;
        assert!(!count.has_connections());

        count.increment();
        assert!(count.has_connections());
        assert_eq!(count.as_usize(), 1);

        count.decrement();
        assert!(!count.has_connections());
    }

    #[test]
    fn test_occurrence_count() {
        let mut count = OccurrenceCount::ONE;
        assert_eq!(count.as_u32(), 1);

        count.increment();
        assert_eq!(count.as_u32(), 2);
    }
}
