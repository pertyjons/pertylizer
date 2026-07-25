//! Signal Monitor parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{Gain, NormalizedValue, Seconds};

// ============================================================================
// SIGNAL MONITOR PARAMETER ENUM (with typed values)
// ============================================================================

/// Signal Monitor parameter with typed value.
///
/// Used for inline signal monitoring in the voice graph.
/// Same parameters as Oscilloscope but for a PolyModule (voice-level).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SignalMonitorParam {
    /// Time scale (seconds per division)
    Time(Seconds),
    /// Input gain
    Gain(Gain),
    /// Trigger level
    Trigger(NormalizedValue),
    /// Freeze display
    Frozen(bool),
}

impl Default for SignalMonitorParam {
    fn default() -> Self {
        Self::Time(Seconds::new(0.01))
    }
}
