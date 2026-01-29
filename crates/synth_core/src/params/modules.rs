//! General module parameter types (Amplifier, Mixer, Sample, Granular, Visualizers).

use serde::{Deserialize, Serialize};

use crate::types::{
    BipolarValue, Gain, Interpolation, Milliseconds, NormalizedValue, Seconds, Semitones,
};

// Re-export LoopMode and ReleaseMode from types module for backwards compatibility
pub use crate::types::{LoopMode, ReleaseMode};

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
            Self::Mute(b) | Self::Limit(b) => {
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
        }
    }
}

impl Default for MixerParam {
    fn default() -> Self {
        Self::Master(Gain::UNITY)
    }
}

// ============================================================================
// SAMPLE PLAYER PARAMETER ENUM (with typed values)
// ============================================================================

/// Sample player parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SamplePlayerParam {
    /// Playback speed (1.0 = normal)
    Speed(NormalizedValue),
    /// Start position (0.0 to 1.0)
    Start(NormalizedValue),
    /// End position (0.0 to 1.0)
    End(NormalizedValue),
    /// Loop mode
    LoopMode(LoopMode),
    /// Loop start position
    LoopStart(NormalizedValue),
    /// Loop end position
    LoopEnd(NormalizedValue),
    /// Loop crossfade time in milliseconds (0-50ms)
    LoopCrossfade(Milliseconds),
    /// Output level
    Level(Gain),
    /// Pan position (-1.0 = left, 0.0 = center, 1.0 = right)
    Pan(BipolarValue),
    /// Velocity sensitivity (0.0 = ignore, 1.0 = full)
    VelocitySensitivity(NormalizedValue),
    /// Release mode (what happens on note-off)
    ReleaseMode(ReleaseMode),
    /// Interpolation method for sample playback
    Interpolation(Interpolation),
}

impl SamplePlayerParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Speed(_) => "Speed",
            Self::Start(_) => "Start",
            Self::End(_) => "End",
            Self::LoopMode(_) => "Loop Mode",
            Self::LoopStart(_) => "Loop Start",
            Self::LoopEnd(_) => "Loop End",
            Self::LoopCrossfade(_) => "Loop X-Fade",
            Self::Level(_) => "Level",
            Self::Pan(_) => "Pan",
            Self::VelocitySensitivity(_) => "Vel Sens",
            Self::ReleaseMode(_) => "Release",
            Self::Interpolation(_) => "Interp",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Speed(v)
            | Self::Start(v)
            | Self::End(v)
            | Self::LoopStart(v)
            | Self::LoopEnd(v)
            | Self::VelocitySensitivity(v) => v.as_f32(),
            Self::LoopMode(m) => m.index() as f32,
            Self::ReleaseMode(m) => m.index() as f32,
            Self::Interpolation(i) => i.index() as f32,
            Self::LoopCrossfade(ms) => ms.as_f32(),
            Self::Level(g) => g.as_f32(),
            Self::Pan(p) => p.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Speed(_) => Self::Speed(NormalizedValue::new(value)),
            Self::Start(_) => Self::Start(NormalizedValue::new(value)),
            Self::End(_) => Self::End(NormalizedValue::new(value)),
            Self::LoopMode(_) => {
                Self::LoopMode(LoopMode::from_index(value as usize).unwrap_or_default())
            }
            Self::LoopStart(_) => Self::LoopStart(NormalizedValue::new(value)),
            Self::LoopEnd(_) => Self::LoopEnd(NormalizedValue::new(value)),
            Self::LoopCrossfade(_) => Self::LoopCrossfade(Milliseconds::new(value)),
            Self::Level(_) => Self::Level(Gain::new(value)),
            Self::Pan(_) => Self::Pan(BipolarValue::new(value)),
            Self::VelocitySensitivity(_) => Self::VelocitySensitivity(NormalizedValue::new(value)),
            Self::ReleaseMode(_) => {
                Self::ReleaseMode(ReleaseMode::from_index(value as usize).unwrap_or_default())
            }
            Self::Interpolation(_) => {
                Self::Interpolation(Interpolation::from_index(value as usize).unwrap_or_default())
            }
        }
    }
}

impl Default for SamplePlayerParam {
    fn default() -> Self {
        Self::Level(Gain::UNITY)
    }
}

// ============================================================================
// GRANULAR PARAMETER ENUM (with typed values)
// ============================================================================

/// Granular synth parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum GranularParam {
    /// Position in sample (0.0 to 1.0)
    Position(NormalizedValue),
    /// Position randomization
    PositionSpread(NormalizedValue),
    /// Grain size in ms
    GrainSize(Milliseconds),
    /// Grain density (grains per second)
    Density(NormalizedValue),
    /// Pitch shift in semitones
    Pitch(Semitones),
    /// Pitch randomization
    PitchSpread(NormalizedValue),
    /// Grain shape/envelope
    Shape(NormalizedValue),
    /// Stereo spread
    Spread(NormalizedValue),
    /// Output level
    Level(Gain),
}

impl GranularParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Position(_) => "Position",
            Self::PositionSpread(_) => "Pos Spread",
            Self::GrainSize(_) => "Grain Size",
            Self::Density(_) => "Density",
            Self::Pitch(_) => "Pitch",
            Self::PitchSpread(_) => "Pitch Spread",
            Self::Shape(_) => "Shape",
            Self::Spread(_) => "Spread",
            Self::Level(_) => "Level",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Position(v)
            | Self::PositionSpread(v)
            | Self::Density(v)
            | Self::PitchSpread(v)
            | Self::Shape(v)
            | Self::Spread(v) => v.as_f32(),
            Self::GrainSize(ms) => ms.as_f32(),
            Self::Pitch(s) => s.as_f32(),
            Self::Level(g) => g.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Position(_) => Self::Position(NormalizedValue::new(value)),
            Self::PositionSpread(_) => Self::PositionSpread(NormalizedValue::new(value)),
            Self::GrainSize(_) => Self::GrainSize(Milliseconds::new(value)),
            Self::Density(_) => Self::Density(NormalizedValue::new(value)),
            Self::Pitch(_) => Self::Pitch(Semitones::new(value)),
            Self::PitchSpread(_) => Self::PitchSpread(NormalizedValue::new(value)),
            Self::Shape(_) => Self::Shape(NormalizedValue::new(value)),
            Self::Spread(_) => Self::Spread(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(Gain::new(value)),
        }
    }
}

impl Default for GranularParam {
    fn default() -> Self {
        Self::Level(Gain::UNITY)
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
