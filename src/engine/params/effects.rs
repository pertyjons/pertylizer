//! Effect-related parameter types.

use serde::{Deserialize, Serialize};

// ============================================================================
// EFFECT MODE ENUMS
// ============================================================================

/// Delay modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum DelayMode {
    #[default]
    Mono,
    Stereo,
    PingPong,
}

impl DelayMode {
    pub const ALL: [Self; 3] = [Self::Mono, Self::Stereo, Self::PingPong];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Mono => "Mono",
            Self::Stereo => "Stereo",
            Self::PingPong => "Ping-Pong",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Mono => "mono",
            Self::Stereo => "stereo",
            Self::PingPong => "ping_pong",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "mono" => Some(Self::Mono),
            "stereo" => Some(Self::Stereo),
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

/// Distortion types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum DistortionMode {
    #[default]
    SoftClip,
    HardClip,
    Tube,
    Foldback,
    Bitcrush,
}

impl DistortionMode {
    pub const ALL: [Self; 5] = [
        Self::SoftClip,
        Self::HardClip,
        Self::Tube,
        Self::Foldback,
        Self::Bitcrush,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::SoftClip => "Soft Clip",
            Self::HardClip => "Hard Clip",
            Self::Tube => "Tube",
            Self::Foldback => "Foldback",
            Self::Bitcrush => "Bitcrush",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::SoftClip => "soft_clip",
            Self::HardClip => "hard_clip",
            Self::Tube => "tube",
            Self::Foldback => "foldback",
            Self::Bitcrush => "bitcrush",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "soft_clip" => Some(Self::SoftClip),
            "hard_clip" => Some(Self::HardClip),
            "tube" => Some(Self::Tube),
            "foldback" => Some(Self::Foldback),
            "bitcrush" => Some(Self::Bitcrush),
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
// EFFECT PARAMETER ENUMS
// ============================================================================

/// Delay effect parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DelayParam {
    /// Delay mode (Mono, Stereo, PingPong)
    Mode,
    /// Delay time in seconds
    Time,
    /// Left channel delay time (for stereo mode)
    TimeLeft,
    /// Right channel delay time (for stereo mode)
    TimeRight,
    /// Feedback amount (0.0 to 1.0)
    Feedback,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
    /// High-frequency damping
    Damping,
    /// Tempo sync enable
    TempoSync,
    /// Sync division in beats (0.25 = 1/16, 0.5 = 1/8, 1.0 = 1/4, 2.0 = 1/2, 4.0 = whole)
    SyncDivision,
}

/// Reverb effect parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReverbParam {
    /// Room size (0.0 to 1.0)
    RoomSize,
    /// Pre-delay time in seconds
    PreDelay,
    /// High-frequency damping (0.0 to 1.0)
    Damping,
    /// Stereo width (0.0 to 1.0)
    Width,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
}

/// Distortion effect parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DistortionParam {
    /// Distortion type/mode
    Mode,
    /// Drive amount (0.0 to 1.0)
    Drive,
    /// Tone/brightness (0.0 to 1.0)
    Tone,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
}

/// Chorus effect parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChorusParam {
    /// Modulation rate in Hz
    Rate,
    /// Modulation depth (0.0 to 1.0)
    Depth,
    /// Delay time in ms
    Delay,
    /// Feedback amount
    Feedback,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
    /// Number of voices
    Voices,
}

/// Phaser effect parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PhaserParam {
    /// LFO rate in Hz
    Rate,
    /// Modulation depth (0.0 to 1.0)
    Depth,
    /// Feedback amount (-1.0 to 1.0)
    Feedback,
    /// Number of allpass stages (2-12)
    Stages,
    /// Center frequency in Hz
    CenterFreq,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
}

/// Flanger effect parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FlangerParam {
    /// LFO rate in Hz
    Rate,
    /// Modulation depth (0.0 to 1.0)
    Depth,
    /// Feedback amount (-1.0 to 1.0)
    Feedback,
    /// Base delay time in ms
    Delay,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
}

/// Compressor effect parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompressorParam {
    /// Threshold in dB (-60 to 0)
    Threshold,
    /// Compression ratio (1:1 to 20:1)
    Ratio,
    /// Attack time in ms
    Attack,
    /// Release time in ms
    Release,
    /// Makeup gain in dB
    Makeup,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
}

/// EQ effect parameters (3-band parametric).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EqParam {
    /// Low band frequency in Hz
    LowFreq,
    /// Low band gain in dB
    LowGain,
    /// Mid band frequency in Hz
    MidFreq,
    /// Mid band gain in dB
    MidGain,
    /// Mid band Q factor
    MidQ,
    /// High band frequency in Hz
    HighFreq,
    /// High band gain in dB
    HighGain,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
}
