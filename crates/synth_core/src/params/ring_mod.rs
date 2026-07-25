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
