//! Effect-related parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{
    BeatDivision, BipolarValue, Decibels, Hertz, MidiNote, Milliseconds, NormalizedValue, Ratio,
    Seconds, Semitones, VoiceCount,
};

use super::FftSizeOption;

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

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| crate::module_traits::ChoiceOption::new(m.id(), m.name()))
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
    /// Variable hardness clipping (arctan-based, continuously adjustable).
    /// Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Effects/104-variable-hardness-clipping-function.rst
    VariableClip,
}

impl DistortionMode {
    pub const ALL: [Self; 6] = [
        Self::SoftClip,
        Self::HardClip,
        Self::Tube,
        Self::Foldback,
        Self::Bitcrush,
        Self::VariableClip,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::SoftClip => "Soft Clip",
            Self::HardClip => "Hard Clip",
            Self::Tube => "Tube",
            Self::Foldback => "Foldback",
            Self::Bitcrush => "Bitcrush",
            Self::VariableClip => "Variable",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::SoftClip => "soft_clip",
            Self::HardClip => "hard_clip",
            Self::Tube => "tube",
            Self::Foldback => "foldback",
            Self::Bitcrush => "bitcrush",
            Self::VariableClip => "variable_clip",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| crate::module_traits::ChoiceOption::new(m.id(), m.name()))
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
            Self::TempoSync(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
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
    fn default() -> Self {
        Self::Time(Seconds::new(0.25))
    }
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
    /// Decay time (0.0 = short, 1.0 = long). FDN reverb.
    Decay(NormalizedValue),
    /// Diffusion amount (0.0 = sparse, 1.0 = dense). FDN reverb.
    Diffusion(NormalizedValue),
    /// Low-cut frequency for reverb tail. FDN reverb.
    LowCut(Hertz),
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
            Self::Decay(_) => "Decay",
            Self::Diffusion(_) => "Diffusion",
            Self::LowCut(_) => "Low Cut",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::RoomSize(v)
            | Self::Damping(v)
            | Self::Width(v)
            | Self::Mix(v)
            | Self::Decay(v)
            | Self::Diffusion(v) => v.as_f32(),
            Self::PreDelay(t) => t.as_f32(),
            Self::LowCut(hz) => hz.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::RoomSize(_) => Self::RoomSize(NormalizedValue::new(value)),
            Self::PreDelay(_) => Self::PreDelay(Seconds::new(value)),
            Self::Damping(_) => Self::Damping(NormalizedValue::new(value)),
            Self::Width(_) => Self::Width(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::Decay(_) => Self::Decay(NormalizedValue::new(value)),
            Self::Diffusion(_) => Self::Diffusion(NormalizedValue::new(value)),
            Self::LowCut(_) => Self::LowCut(Hertz::new(value)),
        }
    }
}

impl Default for ReverbParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::new(0.3))
    }
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
    /// Bit depth for bitcrush mode (1-16 bits).
    BitDepth(NormalizedValue),
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
            Self::BitDepth(_) => "BitDepth",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Mode(m) => m.index() as f32,
            Self::Drive(v) | Self::Tone(v) | Self::Mix(v) | Self::BitDepth(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Mode(_) => {
                Self::Mode(DistortionMode::from_index(value as usize).unwrap_or_default())
            }
            Self::Drive(_) => Self::Drive(NormalizedValue::new(value)),
            Self::Tone(_) => Self::Tone(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::BitDepth(_) => Self::BitDepth(NormalizedValue::new(value)),
        }
    }
}

impl Default for DistortionParam {
    fn default() -> Self {
        Self::Drive(NormalizedValue::new(0.5))
    }
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
    fn default() -> Self {
        Self::Mix(NormalizedValue::new(0.5))
    }
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
    fn default() -> Self {
        Self::Mix(NormalizedValue::new(0.5))
    }
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
    fn default() -> Self {
        Self::Mix(NormalizedValue::new(0.5))
    }
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
    /// Sidechain enabled (use external sidechain input for detection).
    SidechainEnabled(bool),
    /// Sidechain high-pass filter frequency (20-500 Hz).
    SidechainFilter(Hertz),
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
            Self::SidechainEnabled(_) => "Sidechain",
            Self::SidechainFilter(_) => "SC Filter",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Threshold(db) | Self::Makeup(db) => db.as_f32(),
            Self::Ratio(r) => r.as_f32(),
            Self::Attack(ms) | Self::Release(ms) => ms.as_f32(),
            Self::Mix(v) => v.as_f32(),
            Self::SidechainEnabled(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::SidechainFilter(hz) => hz.as_f32(),
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
            Self::SidechainEnabled(_) => Self::SidechainEnabled(value > 0.5),
            Self::SidechainFilter(_) => Self::SidechainFilter(Hertz::new(value)),
        }
    }
}

impl Default for CompressorParam {
    fn default() -> Self {
        Self::Threshold(Decibels::new(-20.0))
    }
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
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}

// ============================================================================
// BBD DELAY PARAMETER ENUM (with typed values)
// ============================================================================

/// BBD (Bucket Brigade Device) analog delay parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BbdDelayParam {
    Time(Seconds),
    Feedback(NormalizedValue),
    Tone(NormalizedValue),
    WowFlutter(NormalizedValue),
    ClockNoise(NormalizedValue),
    Mix(NormalizedValue),
}

impl BbdDelayParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Time(_) => "Time",
            Self::Feedback(_) => "Feedback",
            Self::Tone(_) => "Tone",
            Self::WowFlutter(_) => "Wow/Flutter",
            Self::ClockNoise(_) => "Clock Noise",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Time(t) => t.as_f32(),
            Self::Feedback(v)
            | Self::Tone(v)
            | Self::WowFlutter(v)
            | Self::ClockNoise(v)
            | Self::Mix(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Time(_) => Self::Time(Seconds::new(value)),
            Self::Feedback(_) => Self::Feedback(NormalizedValue::new(value)),
            Self::Tone(_) => Self::Tone(NormalizedValue::new(value)),
            Self::WowFlutter(_) => Self::WowFlutter(NormalizedValue::new(value)),
            Self::ClockNoise(_) => Self::ClockNoise(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for BbdDelayParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::CENTER)
    }
}

// ============================================================================
// MID-SIDE PROCESSING PARAMETER ENUM (with typed values)
// ============================================================================

/// Mid-Side processing parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MidSideParam {
    /// Stereo width (0=mono, 1=normal, 2=extra wide).
    Width(NormalizedValue),
    /// Mid channel gain.
    MidGain(Decibels),
    /// Side channel gain.
    SideGain(Decibels),
    /// Stereo field rotation angle in degrees (-180 to +180).
    /// Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Effects/255-stereo-field-rotation-via-transformation-matrix.rst
    Rotation(BipolarValue),
    /// Dry/wet mix.
    Mix(NormalizedValue),
}

impl MidSideParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Width(_) => "Width",
            Self::MidGain(_) => "Mid Gain",
            Self::SideGain(_) => "Side Gain",
            Self::Rotation(_) => "Rotation",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Width(v) | Self::Mix(v) => v.as_f32(),
            Self::MidGain(db) | Self::SideGain(db) => db.as_f32(),
            Self::Rotation(b) => b.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Width(_) => Self::Width(NormalizedValue::new(value)),
            Self::MidGain(_) => Self::MidGain(Decibels::new(value)),
            Self::SideGain(_) => Self::SideGain(Decibels::new(value)),
            Self::Rotation(_) => Self::Rotation(BipolarValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for MidSideParam {
    fn default() -> Self {
        Self::Width(NormalizedValue::CENTER)
    }
}

// ============================================================================
// LIMITER PARAMETER ENUM (with typed values)
// ============================================================================

/// Brickwall limiter parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LimiterParam {
    /// Ceiling level.
    Ceiling(Decibels),
    /// Look-ahead time.
    LookAhead(Milliseconds),
    /// Release time.
    Release(Milliseconds),
    /// Dry/wet mix.
    Mix(NormalizedValue),
}

impl LimiterParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Ceiling(_) => "Ceiling",
            Self::LookAhead(_) => "Look-Ahead",
            Self::Release(_) => "Release",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Ceiling(db) => db.as_f32(),
            Self::LookAhead(ms) | Self::Release(ms) => ms.as_f32(),
            Self::Mix(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Ceiling(_) => Self::Ceiling(Decibels::new(value)),
            Self::LookAhead(_) => Self::LookAhead(Milliseconds::new(value)),
            Self::Release(_) => Self::Release(Milliseconds::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for LimiterParam {
    fn default() -> Self {
        Self::Ceiling(Decibels::new(-0.3))
    }
}

// ============================================================================
// ENSEMBLE CHORUS (BBD) PARAMETER ENUM
// ============================================================================

/// Juno-style BBD ensemble chorus parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EnsembleChorusParam {
    Rate(Hertz),
    Depth(Milliseconds),
    BaseDelay(Milliseconds),
    Mix(NormalizedValue),
    Tone(NormalizedValue),
    Noise(NormalizedValue),
    StereoWidth(NormalizedValue),
    Voices(VoiceCount),
}

impl EnsembleChorusParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Rate(_) => "Rate",
            Self::Depth(_) => "Depth",
            Self::BaseDelay(_) => "Base Delay",
            Self::Mix(_) => "Mix",
            Self::Tone(_) => "Tone",
            Self::Noise(_) => "Noise",
            Self::StereoWidth(_) => "Stereo Width",
            Self::Voices(_) => "Voices",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Rate(hz) => hz.as_f32(),
            Self::Depth(ms) | Self::BaseDelay(ms) => ms.as_f32(),
            Self::Mix(v) | Self::Tone(v) | Self::Noise(v) | Self::StereoWidth(v) => v.as_f32(),
            Self::Voices(v) => v.as_u8() as f32,
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Depth(_) => Self::Depth(Milliseconds::new(value)),
            Self::BaseDelay(_) => Self::BaseDelay(Milliseconds::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::Tone(_) => Self::Tone(NormalizedValue::new(value)),
            Self::Noise(_) => Self::Noise(NormalizedValue::new(value)),
            Self::StereoWidth(_) => Self::StereoWidth(NormalizedValue::new(value)),
            Self::Voices(_) => Self::Voices(VoiceCount::new(value as u8)),
        }
    }
}

impl Default for EnsembleChorusParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::CENTER)
    }
}

// ============================================================================
// SHIMMER REVERB PARAMETER ENUM
// ============================================================================

/// Shimmer reverb parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ShimmerReverbParam {
    RoomSize(NormalizedValue),
    Decay(NormalizedValue),
    Damping(NormalizedValue),
    PreDelay(Seconds),
    PitchSemitones(Semitones),
    ShimmerMix(NormalizedValue),
    Mix(NormalizedValue),
}

impl ShimmerReverbParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::RoomSize(_) => "Room Size",
            Self::Decay(_) => "Decay",
            Self::Damping(_) => "Damping",
            Self::PreDelay(_) => "Pre-Delay",
            Self::PitchSemitones(_) => "Pitch",
            Self::ShimmerMix(_) => "Shimmer",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::RoomSize(v)
            | Self::Decay(v)
            | Self::Damping(v)
            | Self::ShimmerMix(v)
            | Self::Mix(v) => v.as_f32(),
            Self::PreDelay(t) => t.as_f32(),
            Self::PitchSemitones(s) => s.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::RoomSize(_) => Self::RoomSize(NormalizedValue::new(value)),
            Self::Decay(_) => Self::Decay(NormalizedValue::new(value)),
            Self::Damping(_) => Self::Damping(NormalizedValue::new(value)),
            Self::PreDelay(_) => Self::PreDelay(Seconds::new(value)),
            Self::PitchSemitones(_) => Self::PitchSemitones(Semitones::new(value)),
            Self::ShimmerMix(_) => Self::ShimmerMix(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for ShimmerReverbParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::new(0.4))
    }
}

// ============================================================================
// GRANULAR FX PARAMETER ENUM
// ============================================================================

/// Granular FX parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum GranularFxParam {
    BufferTime(Seconds),
    GrainSize(Milliseconds),
    Density(NormalizedValue),
    Position(NormalizedValue),
    PositionSpread(NormalizedValue),
    PitchSpread(NormalizedValue),
    PanSpread(NormalizedValue),
    Freeze(bool),
    Mix(NormalizedValue),
}

impl GranularFxParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::BufferTime(_) => "Buffer",
            Self::GrainSize(_) => "Grain Size",
            Self::Density(_) => "Density",
            Self::Position(_) => "Position",
            Self::PositionSpread(_) => "Pos Spread",
            Self::PitchSpread(_) => "Pitch Spread",
            Self::PanSpread(_) => "Pan Spread",
            Self::Freeze(_) => "Freeze",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::BufferTime(t) => t.as_f32(),
            Self::GrainSize(ms) => ms.as_f32(),
            Self::Density(v)
            | Self::Position(v)
            | Self::PositionSpread(v)
            | Self::PitchSpread(v)
            | Self::PanSpread(v)
            | Self::Mix(v) => v.as_f32(),
            Self::Freeze(b) => {
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
            Self::BufferTime(_) => Self::BufferTime(Seconds::new(value)),
            Self::GrainSize(_) => Self::GrainSize(Milliseconds::new(value)),
            Self::Density(_) => Self::Density(NormalizedValue::new(value)),
            Self::Position(_) => Self::Position(NormalizedValue::new(value)),
            Self::PositionSpread(_) => Self::PositionSpread(NormalizedValue::new(value)),
            Self::PitchSpread(_) => Self::PitchSpread(NormalizedValue::new(value)),
            Self::PanSpread(_) => Self::PanSpread(NormalizedValue::new(value)),
            Self::Freeze(_) => Self::Freeze(value > 0.5),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for GranularFxParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}

// ============================================================================
// SPECTRAL BLUR PARAMETER ENUM
// ============================================================================

/// Spectral blur/smear parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SpectralBlurParam {
    FftSize(FftSizeOption),
    BlurTime(NormalizedValue),
    BlurFreq(NormalizedValue),
    Freeze(bool),
    Mix(NormalizedValue),
}

impl SpectralBlurParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::FftSize(_) => "FFT Size",
            Self::BlurTime(_) => "Blur Time",
            Self::BlurFreq(_) => "Blur Freq",
            Self::Freeze(_) => "Freeze",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::FftSize(f) => f.index() as f32,
            Self::BlurTime(v) | Self::BlurFreq(v) | Self::Mix(v) => v.as_f32(),
            Self::Freeze(b) => {
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
            Self::FftSize(_) => Self::FftSize(FftSizeOption::from_index(value as usize)),
            Self::BlurTime(_) => Self::BlurTime(NormalizedValue::new(value)),
            Self::BlurFreq(_) => Self::BlurFreq(NormalizedValue::new(value)),
            Self::Freeze(_) => Self::Freeze(value > 0.5),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for SpectralBlurParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}

// ============================================================================
// MODAL RESONATOR PARAMETER ENUM
// ============================================================================

/// Modal resonator bank parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ModalResonatorParam {
    BaseNote(MidiNote),
    Spread(NormalizedValue),
    Modes(VoiceCount),
    Decay(NormalizedValue),
    Brightness(NormalizedValue),
    Mix(NormalizedValue),
}

impl ModalResonatorParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::BaseNote(_) => "Base Note",
            Self::Spread(_) => "Spread",
            Self::Modes(_) => "Modes",
            Self::Decay(_) => "Decay",
            Self::Brightness(_) => "Brightness",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::BaseNote(n) => n.as_u8() as f32,
            Self::Spread(v) | Self::Decay(v) | Self::Brightness(v) | Self::Mix(v) => v.as_f32(),
            Self::Modes(m) => m.as_u8() as f32,
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::BaseNote(_) => Self::BaseNote(MidiNote::new(value as u8)),
            Self::Spread(_) => Self::Spread(NormalizedValue::new(value)),
            Self::Modes(_) => Self::Modes(VoiceCount::new(value as u8)),
            Self::Decay(_) => Self::Decay(NormalizedValue::new(value)),
            Self::Brightness(_) => Self::Brightness(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for ModalResonatorParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}

// ============================================================================
// REVERSE/GATE REVERB MODE ENUMS
// ============================================================================

/// Playback mode for reverse/gate reverb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ReverseGateMode {
    #[default]
    Reverse,
    Gate,
    Stutter,
}

impl ReverseGateMode {
    pub const ALL: [Self; 3] = [Self::Reverse, Self::Gate, Self::Stutter];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Reverse => "Reverse",
            Self::Gate => "Gate",
            Self::Stutter => "Stutter",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Reverse => "reverse",
            Self::Gate => "gate",
            Self::Stutter => "stutter",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| crate::module_traits::ChoiceOption::new(m.id(), m.name()))
            .collect()
    }
}

/// Trigger mode for reverse/gate reverb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ReverseGateTrigger {
    #[default]
    Periodic,
    Threshold,
}

impl ReverseGateTrigger {
    pub const ALL: [Self; 2] = [Self::Periodic, Self::Threshold];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Periodic => "Periodic",
            Self::Threshold => "Threshold",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Periodic => "periodic",
            Self::Threshold => "threshold",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| crate::module_traits::ChoiceOption::new(m.id(), m.name()))
            .collect()
    }
}

// ============================================================================
// REVERSE/GATE REVERB PARAMETER ENUM
// ============================================================================

/// Reverse/gate reverb parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ReverseGateReverbParam {
    WindowTime(Milliseconds),
    Mode(ReverseGateMode),
    Trigger(ReverseGateTrigger),
    Threshold(Decibels),
    GateTime(Milliseconds),
    Mix(NormalizedValue),
}

impl ReverseGateReverbParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::WindowTime(_) => "Window",
            Self::Mode(_) => "Mode",
            Self::Trigger(_) => "Trigger",
            Self::Threshold(_) => "Threshold",
            Self::GateTime(_) => "Gate Time",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::WindowTime(ms) | Self::GateTime(ms) => ms.as_f32(),
            Self::Mode(m) => m.index() as f32,
            Self::Trigger(t) => t.index() as f32,
            Self::Threshold(db) => db.as_f32(),
            Self::Mix(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::WindowTime(_) => Self::WindowTime(Milliseconds::new(value)),
            Self::Mode(_) => {
                Self::Mode(ReverseGateMode::from_index(value as usize).unwrap_or_default())
            }
            Self::Trigger(_) => {
                Self::Trigger(ReverseGateTrigger::from_index(value as usize).unwrap_or_default())
            }
            Self::Threshold(_) => Self::Threshold(Decibels::new(value)),
            Self::GateTime(_) => Self::GateTime(Milliseconds::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for ReverseGateReverbParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}

// ============================================================================
// TILT EQ PARAMETERS
// ============================================================================

/// Tilt EQ parameter with typed value.
///
/// Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Filters/267-simple-tilt-equalizer.rst
/// From the Music-DSP Source Code Archive (https://www.musicdsp.org/)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TiltEqParam {
    /// Tilt amount (-1.0 = dark, 0.0 = flat, 1.0 = bright)
    Tilt(BipolarValue),
    /// Center frequency (pivot point)
    CenterFreq(Hertz),
    /// Dry/wet mix
    Mix(NormalizedValue),
}

impl TiltEqParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Tilt(_) => "Tilt",
            Self::CenterFreq(_) => "Center",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Tilt(b) => b.as_f32(),
            Self::CenterFreq(hz) => hz.as_f32(),
            Self::Mix(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Tilt(_) => Self::Tilt(BipolarValue::new(value)),
            Self::CenterFreq(_) => Self::CenterFreq(Hertz::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for TiltEqParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}

// ============================================================================
// UNIVIBE PARAMETERS
// ============================================================================

/// Univibe effect parameter with typed value.
///
/// Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Effects/277-univox-univibe-emulator.rst
/// From the Music-DSP Source Code Archive (https://www.musicdsp.org/)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum UnivibeParam {
    /// LFO rate (speed of sweep)
    Rate(Hertz),
    /// Depth of modulation (0.0 to 1.0)
    Depth(NormalizedValue),
    /// Feedback amount (0.0 to 0.95)
    Feedback(NormalizedValue),
    /// Dry/wet mix
    Mix(NormalizedValue),
}

impl UnivibeParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Rate(_) => "Rate",
            Self::Depth(_) => "Depth",
            Self::Feedback(_) => "Feedback",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Rate(hz) => hz.as_f32(),
            Self::Depth(v) | Self::Feedback(v) | Self::Mix(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
            Self::Feedback(_) => Self::Feedback(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for UnivibeParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::new(0.5))
    }
}

// ============================================================================
// CROSSOVER SPLITTER PARAMETERS
// ============================================================================

/// Crossover splitter parameter with typed value.
///
/// Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Filters/266-4th-order-linkwitz-riley-filters.rst
/// From the Music-DSP Source Code Archive (https://www.musicdsp.org/)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CrossoverParam {
    /// Crossover frequency (Hz)
    Frequency(Hertz),
    /// Low band gain
    LowGain(NormalizedValue),
    /// High band gain
    HighGain(NormalizedValue),
    /// Dry/wet mix
    Mix(NormalizedValue),
}

impl CrossoverParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Frequency(_) => "Frequency",
            Self::LowGain(_) => "Low Gain",
            Self::HighGain(_) => "High Gain",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Frequency(hz) => hz.as_f32(),
            Self::LowGain(v) | Self::HighGain(v) | Self::Mix(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Frequency(_) => Self::Frequency(Hertz::new(value)),
            Self::LowGain(_) => Self::LowGain(NormalizedValue::new(value)),
            Self::HighGain(_) => Self::HighGain(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for CrossoverParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}

// ============================================================================
// VOCODER PARAMETERS
// ============================================================================

/// LPC Vocoder parameter with typed value.
///
/// Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Analysis/137-lpc-analysis-autocorrelation-levinson-durbin-recursion.rst
/// From the Music-DSP Source Code Archive (https://www.musicdsp.org/)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum VocoderParam {
    /// LPC analysis order (4-32)
    Order(NormalizedValue),
    /// Analysis window size in milliseconds
    WindowSize(Milliseconds),
    /// Dry/wet mix
    Mix(NormalizedValue),
}

impl VocoderParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Order(_) => "Order",
            Self::WindowSize(_) => "Window",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Order(v) | Self::Mix(v) => v.as_f32(),
            Self::WindowSize(ms) => ms.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Order(_) => Self::Order(NormalizedValue::new(value)),
            Self::WindowSize(_) => Self::WindowSize(Milliseconds::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for VocoderParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}
