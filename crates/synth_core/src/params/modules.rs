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
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Level(_) => "Level",
            Self::Pan(_) => "Pan",
            Self::CvBipolar(_) => "CV Bipolar",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Level(g) => g.as_f32(),
            Self::Pan(p) => p.as_f32(),
            Self::CvBipolar(b) => {
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
            Self::Level(_) => Self::Level(Gain::new(value)),
            Self::Pan(_) => Self::Pan(BipolarValue::new(value)),
            Self::CvBipolar(_) => Self::CvBipolar(value > 0.5),
        }
    }

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

impl MixerParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Input1(_) => "Input 1",
            Self::Input2(_) => "Input 2",
            Self::Input3(_) => "Input 3",
            Self::Input4(_) => "Input 4",
            Self::Input5(_) => "Input 5",
            Self::Input6(_) => "Input 6",
            Self::Input7(_) => "Input 7",
            Self::Input8(_) => "Input 8",
            Self::Master(_) => "Master",
            Self::Mute(_) => "Mute",
            Self::Limit(_) => "Limit",
            Self::Dither(_) => "Dither",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Input1(g)
            | Self::Input2(g)
            | Self::Input3(g)
            | Self::Input4(g)
            | Self::Input5(g)
            | Self::Input6(g)
            | Self::Input7(g)
            | Self::Input8(g)
            | Self::Master(g) => g.as_f32(),
            Self::Mute(b) | Self::Limit(b) | Self::Dither(b) => {
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
            Self::Input1(_) => Self::Input1(Gain::new(value)),
            Self::Input2(_) => Self::Input2(Gain::new(value)),
            Self::Input3(_) => Self::Input3(Gain::new(value)),
            Self::Input4(_) => Self::Input4(Gain::new(value)),
            Self::Input5(_) => Self::Input5(Gain::new(value)),
            Self::Input6(_) => Self::Input6(Gain::new(value)),
            Self::Input7(_) => Self::Input7(Gain::new(value)),
            Self::Input8(_) => Self::Input8(Gain::new(value)),
            Self::Master(_) => Self::Master(Gain::new(value)),
            Self::Mute(_) => Self::Mute(value > 0.5),
            Self::Limit(_) => Self::Limit(value > 0.5),
            Self::Dither(_) => Self::Dither(value > 0.5),
        }
    }
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

impl OscilloscopeParam {
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

impl LevelMeterParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::PeakHold(_) => "Peak Hold",
            Self::DecayRate(_) => "Decay Rate",
            Self::ShowRms(_) => "Show RMS",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::PeakHold(t) => t.as_f32(),
            Self::DecayRate(v) => v.as_f32(),
            Self::ShowRms(b) => {
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
            Self::PeakHold(_) => Self::PeakHold(Seconds::new(value)),
            Self::DecayRate(_) => Self::DecayRate(NormalizedValue::new(value)),
            Self::ShowRms(_) => Self::ShowRms(value > 0.5),
        }
    }
}

impl Default for LevelMeterParam {
    fn default() -> Self {
        Self::PeakHold(Seconds::new(1.0))
    }
}
