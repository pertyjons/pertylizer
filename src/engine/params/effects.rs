//! Effect-related parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{BeatDivision, BipolarValue, Decibels, Hertz, Milliseconds, NormalizedValue, Ratio, Seconds, VoiceCount};

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

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::modules::core::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| crate::modules::core::ChoiceOption::new(m.id(), m.name()))
            .collect()
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

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::modules::core::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| crate::modules::core::ChoiceOption::new(m.id(), m.name()))
            .collect()
    }
}

// ============================================================================
// DELAY PARAMETER ENUM (with typed values)
// ============================================================================

/// Delay effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DelayParam {
    Mode(DelayMode),
    Time(Seconds),
    TimeLeft(Seconds),
    TimeRight(Seconds),
    Feedback(NormalizedValue),
    Mix(NormalizedValue),
    Damping(NormalizedValue),
    TempoSync(bool),
    SyncDivision(BeatDivision),
}

impl DelayParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Mode(_) => "Mode",
            Self::Time(_) => "Time",
            Self::TimeLeft(_) => "Time L",
            Self::TimeRight(_) => "Time R",
            Self::Feedback(_) => "Feedback",
            Self::Mix(_) => "Mix",
            Self::Damping(_) => "Damping",
            Self::TempoSync(_) => "Tempo Sync",
            Self::SyncDivision(_) => "Division",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Mode(m) => m.index() as f32,
            Self::Time(t) | Self::TimeLeft(t) | Self::TimeRight(t) => t.as_f32(),
            Self::Feedback(v) | Self::Mix(v) | Self::Damping(v) => v.as_f32(),
            Self::TempoSync(b) => if *b { 1.0 } else { 0.0 },
            Self::SyncDivision(d) => d.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Mode(_) => Self::Mode(DelayMode::from_index(value as usize).unwrap_or_default()),
            Self::Time(_) => Self::Time(Seconds::new(value)),
            Self::TimeLeft(_) => Self::TimeLeft(Seconds::new(value)),
            Self::TimeRight(_) => Self::TimeRight(Seconds::new(value)),
            Self::Feedback(_) => Self::Feedback(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::Damping(_) => Self::Damping(NormalizedValue::new(value)),
            Self::TempoSync(_) => Self::TempoSync(value > 0.5),
            Self::SyncDivision(_) => Self::SyncDivision(BeatDivision::new(value)),
        }
    }
}

impl Default for DelayParam {
    fn default() -> Self { Self::Time(Seconds::new(0.25)) }
}

// ============================================================================
// REVERB PARAMETER ENUM (with typed values)
// ============================================================================

/// Reverb effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ReverbParam {
    RoomSize(NormalizedValue),
    PreDelay(Seconds),
    Damping(NormalizedValue),
    Width(NormalizedValue),
    Mix(NormalizedValue),
}

impl ReverbParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::RoomSize(_) => "Room Size",
            Self::PreDelay(_) => "Pre-Delay",
            Self::Damping(_) => "Damping",
            Self::Width(_) => "Width",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::RoomSize(v) | Self::Damping(v) | Self::Width(v) | Self::Mix(v) => v.as_f32(),
            Self::PreDelay(t) => t.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::RoomSize(_) => Self::RoomSize(NormalizedValue::new(value)),
            Self::PreDelay(_) => Self::PreDelay(Seconds::new(value)),
            Self::Damping(_) => Self::Damping(NormalizedValue::new(value)),
            Self::Width(_) => Self::Width(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for ReverbParam {
    fn default() -> Self { Self::Mix(NormalizedValue::new(0.3)) }
}

// ============================================================================
// DISTORTION PARAMETER ENUM (with typed values)
// ============================================================================

/// Distortion effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DistortionParam {
    Mode(DistortionMode),
    Drive(NormalizedValue),
    Tone(NormalizedValue),
    Mix(NormalizedValue),
}

impl DistortionParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Mode(_) => "Mode",
            Self::Drive(_) => "Drive",
            Self::Tone(_) => "Tone",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Mode(m) => m.index() as f32,
            Self::Drive(v) | Self::Tone(v) | Self::Mix(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Mode(_) => Self::Mode(DistortionMode::from_index(value as usize).unwrap_or_default()),
            Self::Drive(_) => Self::Drive(NormalizedValue::new(value)),
            Self::Tone(_) => Self::Tone(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for DistortionParam {
    fn default() -> Self { Self::Drive(NormalizedValue::new(0.5)) }
}

// ============================================================================
// CHORUS PARAMETER ENUM (with typed values)
// ============================================================================

/// Chorus effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ChorusParam {
    Rate(Hertz),
    Depth(NormalizedValue),
    Delay(Milliseconds),
    Feedback(NormalizedValue),
    Mix(NormalizedValue),
    Voices(VoiceCount),
}

impl ChorusParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Rate(_) => "Rate",
            Self::Depth(_) => "Depth",
            Self::Delay(_) => "Delay",
            Self::Feedback(_) => "Feedback",
            Self::Mix(_) => "Mix",
            Self::Voices(_) => "Voices",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Rate(hz) => hz.as_f32(),
            Self::Depth(v) | Self::Feedback(v) | Self::Mix(v) => v.as_f32(),
            Self::Delay(ms) => ms.as_f32(),
            Self::Voices(v) => v.as_u8() as f32,
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
            Self::Delay(_) => Self::Delay(Milliseconds::new(value)),
            Self::Feedback(_) => Self::Feedback(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::Voices(_) => Self::Voices(VoiceCount::new(value as u8)),
        }
    }
}

impl Default for ChorusParam {
    fn default() -> Self { Self::Mix(NormalizedValue::new(0.5)) }
}

// ============================================================================
// PHASER PARAMETER ENUM (with typed values)
// ============================================================================

/// Phaser effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PhaserParam {
    Rate(Hertz),
    Depth(NormalizedValue),
    Feedback(BipolarValue),
    Stages(u8),
    CenterFreq(Hertz),
    Mix(NormalizedValue),
}

impl PhaserParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Rate(_) => "Rate",
            Self::Depth(_) => "Depth",
            Self::Feedback(_) => "Feedback",
            Self::Stages(_) => "Stages",
            Self::CenterFreq(_) => "Center Freq",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Rate(hz) | Self::CenterFreq(hz) => hz.as_f32(),
            Self::Depth(v) | Self::Mix(v) => v.as_f32(),
            Self::Feedback(b) => b.as_f32(),
            Self::Stages(s) => *s as f32,
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
            Self::Feedback(_) => Self::Feedback(BipolarValue::new(value)),
            Self::Stages(_) => Self::Stages(value as u8),
            Self::CenterFreq(_) => Self::CenterFreq(Hertz::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for PhaserParam {
    fn default() -> Self { Self::Mix(NormalizedValue::new(0.5)) }
}

// ============================================================================
// FLANGER PARAMETER ENUM (with typed values)
// ============================================================================

/// Flanger effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FlangerParam {
    Rate(Hertz),
    Depth(NormalizedValue),
    Feedback(BipolarValue),
    Delay(Milliseconds),
    Mix(NormalizedValue),
}

impl FlangerParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Rate(_) => "Rate",
            Self::Depth(_) => "Depth",
            Self::Feedback(_) => "Feedback",
            Self::Delay(_) => "Delay",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Rate(hz) => hz.as_f32(),
            Self::Depth(v) | Self::Mix(v) => v.as_f32(),
            Self::Feedback(b) => b.as_f32(),
            Self::Delay(ms) => ms.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
            Self::Feedback(_) => Self::Feedback(BipolarValue::new(value)),
            Self::Delay(_) => Self::Delay(Milliseconds::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for FlangerParam {
    fn default() -> Self { Self::Mix(NormalizedValue::new(0.5)) }
}

// ============================================================================
// COMPRESSOR PARAMETER ENUM (with typed values)
// ============================================================================

/// Compressor effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CompressorParam {
    Threshold(Decibels),
    Ratio(Ratio),
    Attack(Milliseconds),
    Release(Milliseconds),
    Makeup(Decibels),
    Mix(NormalizedValue),
}

impl CompressorParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Threshold(_) => "Threshold",
            Self::Ratio(_) => "Ratio",
            Self::Attack(_) => "Attack",
            Self::Release(_) => "Release",
            Self::Makeup(_) => "Makeup",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Threshold(db) | Self::Makeup(db) => db.as_f32(),
            Self::Ratio(r) => r.as_f32(),
            Self::Attack(ms) | Self::Release(ms) => ms.as_f32(),
            Self::Mix(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Threshold(_) => Self::Threshold(Decibels::new(value)),
            Self::Ratio(_) => Self::Ratio(Ratio::new(value)),
            Self::Attack(_) => Self::Attack(Milliseconds::new(value)),
            Self::Release(_) => Self::Release(Milliseconds::new(value)),
            Self::Makeup(_) => Self::Makeup(Decibels::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for CompressorParam {
    fn default() -> Self { Self::Threshold(Decibels::new(-20.0)) }
}

// ============================================================================
// EQ PARAMETER ENUM (with typed values)
// ============================================================================

/// EQ effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EqParam {
    LowFreq(Hertz),
    LowGain(Decibels),
    MidFreq(Hertz),
    MidGain(Decibels),
    MidQ(NormalizedValue),
    HighFreq(Hertz),
    HighGain(Decibels),
    Mix(NormalizedValue),
}

impl EqParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::LowFreq(_) => "Low Freq",
            Self::LowGain(_) => "Low Gain",
            Self::MidFreq(_) => "Mid Freq",
            Self::MidGain(_) => "Mid Gain",
            Self::MidQ(_) => "Mid Q",
            Self::HighFreq(_) => "High Freq",
            Self::HighGain(_) => "High Gain",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::LowFreq(hz) | Self::MidFreq(hz) | Self::HighFreq(hz) => hz.as_f32(),
            Self::LowGain(db) | Self::MidGain(db) | Self::HighGain(db) => db.as_f32(),
            Self::MidQ(v) | Self::Mix(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::LowFreq(_) => Self::LowFreq(Hertz::new(value)),
            Self::LowGain(_) => Self::LowGain(Decibels::new(value)),
            Self::MidFreq(_) => Self::MidFreq(Hertz::new(value)),
            Self::MidGain(_) => Self::MidGain(Decibels::new(value)),
            Self::MidQ(_) => Self::MidQ(NormalizedValue::new(value)),
            Self::HighFreq(_) => Self::HighFreq(Hertz::new(value)),
            Self::HighGain(_) => Self::HighGain(Decibels::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for EqParam {
    fn default() -> Self { Self::Mix(NormalizedValue::MAX) }
}
