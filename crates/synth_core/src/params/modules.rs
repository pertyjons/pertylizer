//! General module parameter types (Amplifier, Mixer, Visualizers).

use serde::{Deserialize, Serialize};

use crate::types::{BipolarValue, Gain, NormalizedValue, Seconds};

// ============================================================================
// AMPLIFIER PARAMETER ENUM (with typed values)
// ============================================================================

/// Amplifier/VCA parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum AmplifierParam {
    /// Output level (0.0 to 1.0)
    Level(Gain),
    /// Stereo pan (-1.0 to 1.0)
    Pan(BipolarValue),
    /// Bipolar CV mode (allows negative gain for ring modulation)
    CvBipolar(bool),
}

impl AmplifierParam {
    pub fn level_default() -> Self {
        Self::Level(Gain::UNITY)
    }
    pub fn pan_default() -> Self {
        Self::Pan(BipolarValue::CENTER)
    }
}

impl Default for AmplifierParam {
    fn default() -> Self {
        Self::Level(Gain::UNITY)
    }
}

// ============================================================================
// MIXER PARAMETER ENUM (with typed values)
// ============================================================================

/// Mixer parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MixerParam {
    /// Input 1 level
    Input1(Gain),
    /// Input 2 level
    Input2(Gain),
    /// Input 3 level
    Input3(Gain),
    /// Input 4 level
    Input4(Gain),
    /// Input 5 level
    Input5(Gain),
    /// Input 6 level
    Input6(Gain),
    /// Input 7 level
    Input7(Gain),
    /// Input 8 level
    Input8(Gain),
    /// Master output level
    Master(Gain),
    /// Mute toggle
    Mute(bool),
    /// Limiter enable toggle
    Limit(bool),
    /// Dither enable toggle
    Dither(bool),
}

impl Default for MixerParam {
    fn default() -> Self {
        Self::Master(Gain::UNITY)
    }
}

// ============================================================================
// OSCILLOSCOPE PARAMETER ENUM (with typed values)
// ============================================================================

/// Oscilloscope visualizer parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum OscilloscopeParam {
    /// Time scale (seconds per division)
    Time(Seconds),
    /// Input gain
    Gain(Gain),
    /// Trigger level
    Trigger(NormalizedValue),
    /// Freeze display
    Frozen(bool),
}

impl Default for OscilloscopeParam {
    fn default() -> Self {
        Self::Time(Seconds::new(0.01))
    }
}

// ============================================================================
// LEVEL METER PARAMETER ENUM (with typed values)
// ============================================================================

/// Level meter visualizer parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LevelMeterParam {
    /// Peak hold time (seconds)
    PeakHold(Seconds),
    /// Meter decay rate
    DecayRate(NormalizedValue),
    /// Show RMS vs peak
    ShowRms(bool),
}

impl Default for LevelMeterParam {
    fn default() -> Self {
        Self::PeakHold(Seconds::new(1.0))
    }
}
