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

impl SignalMonitorParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Time(_) => "Time",
            Self::Gain(_) => "Gain",
            Self::Trigger(_) => "Trigger",
            Self::Frozen(_) => "Frozen",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Time(t) => t.as_f32(),
            Self::Gain(g) => g.as_f32(),
            Self::Trigger(v) => v.as_f32(),
            Self::Frozen(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Time(_) => Self::Time(Seconds::new(value)),
            Self::Gain(_) => Self::Gain(Gain::new(value)),
            Self::Trigger(_) => Self::Trigger(NormalizedValue::new(value)),
            Self::Frozen(_) => Self::Frozen(value > 0.5),
        }
    }
}

impl Default for SignalMonitorParam {
    fn default() -> Self {
        Self::Time(Seconds::new(0.01))
    }
}
