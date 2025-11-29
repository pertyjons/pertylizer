//! General module parameter types (Amplifier, Mixer, Sample, Granular, Visualizers).

use serde::{Deserialize, Serialize};

// ============================================================================
// LOOP MODE ENUM
// ============================================================================

/// Loop modes for sample playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum LoopMode {
    #[default]
    Off,
    Forward,
    Backward,
    PingPong,
}

impl LoopMode {
    pub const ALL: [Self; 4] = [Self::Off, Self::Forward, Self::Backward, Self::PingPong];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Forward => "Forward",
            Self::Backward => "Backward",
            Self::PingPong => "Ping-Pong",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Forward => "forward",
            Self::Backward => "backward",
            Self::PingPong => "ping_pong",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "off" => Some(Self::Off),
            "forward" => Some(Self::Forward),
            "backward" => Some(Self::Backward),
            "ping_pong" => Some(Self::PingPong),
            _ => None,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }
}

// ============================================================================
// MODULE PARAMETER ENUMS
// ============================================================================

/// Amplifier/VCA parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AmplifierParam {
    /// Output level (0.0 to 1.0)
    Level,
    /// Stereo pan (-1.0 to 1.0)
    Pan,
}

/// Mixer parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MixerParam {
    /// Input 1 level
    Input1,
    /// Input 2 level
    Input2,
    /// Input 3 level
    Input3,
    /// Input 4 level
    Input4,
    /// Master output level
    Master,
    /// Mute toggle
    Mute,
    /// Limiter enable toggle
    Limit,
}

/// Sample player parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SamplePlayerParam {
    /// Playback speed (1.0 = normal)
    Speed,
    /// Start position (0.0 to 1.0)
    Start,
    /// End position (0.0 to 1.0)
    End,
    /// Loop mode
    LoopMode,
    /// Loop start position
    LoopStart,
    /// Loop end position
    LoopEnd,
    /// Output level
    Level,
}

/// Granular synth parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GranularParam {
    /// Position in sample (0.0 to 1.0)
    Position,
    /// Position randomization
    PositionSpread,
    /// Grain size in ms
    GrainSize,
    /// Grain density (grains per second)
    Density,
    /// Pitch shift in semitones
    Pitch,
    /// Pitch randomization
    PitchSpread,
    /// Grain shape/envelope
    Shape,
    /// Stereo spread
    Spread,
    /// Output level
    Level,
}

/// Oscilloscope visualizer parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OscilloscopeParam {
    /// Time scale (seconds per division)
    Time,
    /// Input gain
    Gain,
    /// Trigger level
    Trigger,
    /// Freeze display
    Frozen,
}

/// Level meter visualizer parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LevelMeterParam {
    /// Peak hold time (seconds)
    PeakHold,
    /// Meter decay rate
    DecayRate,
    /// Show RMS vs peak
    ShowRms,
}
