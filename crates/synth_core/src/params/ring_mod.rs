//! Ring modulation parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{Hertz, NormalizedValue};

use super::oscillators::Waveform;

// ============================================================================
// RING MOD PARAMETER ENUM (with typed values)
// ============================================================================

/// Ring modulation parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RingModParam {
    /// Carrier oscillator frequency (0.1 Hz to 20000 Hz)
    CarrierFreq(Hertz),
    /// Carrier waveform (sine, triangle, sawtooth, square, pulse)
    CarrierWaveform(Waveform),
    /// Dry/wet mix (0.0 = fully dry, 1.0 = fully ring modulated)
    Mix(NormalizedValue),
    /// Frequency ratio relative to note (0.0-1.0, maps to 0.25x-4.0x)
    FreqRatio(NormalizedValue),
    /// Keyboard tracking amount (0.0 = fixed freq, 1.0 = full tracking)
    TrackKeyboard(NormalizedValue),
}

impl RingModParam {
    /// Check if two parameters are the same kind (ignoring values).
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// Get the parameter name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::CarrierFreq(_) => "Carrier Freq",
            Self::CarrierWaveform(_) => "Carrier Wave",
            Self::Mix(_) => "Mix",
            Self::FreqRatio(_) => "Freq Ratio",
            Self::TrackKeyboard(_) => "Key Track",
        }
    }

    /// Get the value as f32 (for GUI).
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::CarrierFreq(h) => h.as_f32(),
            Self::CarrierWaveform(w) => w.index() as f32,
            Self::Mix(v) => v.as_f32(),
            Self::FreqRatio(v) => v.as_f32(),
            Self::TrackKeyboard(v) => v.as_f32(),
        }
    }

    /// Create the same parameter variant with a new f32 value (for GUI).
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::CarrierFreq(_) => Self::CarrierFreq(Hertz::new(value)),
            Self::CarrierWaveform(_) => {
                Self::CarrierWaveform(Waveform::from_index(value as usize).unwrap_or_default())
            }
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::FreqRatio(_) => Self::FreqRatio(NormalizedValue::new(value)),
            Self::TrackKeyboard(_) => Self::TrackKeyboard(NormalizedValue::new(value)),
        }
    }

    /// Default templates
    pub fn carrier_freq_default() -> Self {
        Self::CarrierFreq(Hertz::new(440.0))
    }
    pub fn carrier_waveform_default() -> Self {
        Self::CarrierWaveform(Waveform::Sine)
    }
    pub fn mix_default() -> Self {
        Self::Mix(NormalizedValue::new(0.5))
    }
    pub fn freq_ratio_default() -> Self {
        Self::FreqRatio(NormalizedValue::new(0.5))
    }
    pub fn track_keyboard_default() -> Self {
        Self::TrackKeyboard(NormalizedValue::MIN)
    }
}

impl Default for RingModParam {
    fn default() -> Self {
        Self::carrier_freq_default()
    }
}
