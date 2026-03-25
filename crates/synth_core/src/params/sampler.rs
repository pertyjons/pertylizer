//! Sampler module parameters.
//!
//! Defined in `synth_core` (not `synth_sampler`) to avoid circular dependencies,
//! since `Param` lives in `synth_core`.

use serde::{Deserialize, Serialize};

use crate::{Cents, Gain, NormalizedValue};

/// Unique identifier for a sample in the library.
/// Duplicated here to avoid `synth_core` depending on `synth_sampler`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct SampleId(pub u64);

/// Playback mode — how the sample responds to `NoteOn`/`NoteOff`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SamplerPlayMode {
    /// Play the full sample regardless of `NoteOff` (drums, one-hit SFX).
    OneShot,
    /// Play until `NoteOff`, then stop. Default for melodic samples.
    #[default]
    Sustain,
    /// Loop within the loop region until `NoteOff`, then release.
    Loop,
}

/// Playback direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PlayDirection {
    #[default]
    Forward,
    Reverse,
    PingPong,
}

/// Parameters for the Sampler module.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SamplerParam {
    /// Which sample to play (by `SampleId`).
    SampleSelect(SampleId),
    /// Pitch tracking — follow MIDI note or play at fixed pitch.
    PitchTracking(bool),
    /// Volume/gain.
    Level(Gain),
    /// Playback mode.
    PlayMode(SamplerPlayMode),
    /// Playback direction.
    Direction(PlayDirection),
    /// Velocity sensitivity (how much velocity affects volume).
    VelocitySensitivity(NormalizedValue),
    /// Fine-tune in cents (-100 to +100).
    FineTune(Cents),
    /// Playback start offset (0.0 = beginning of crop, 1.0 = end).
    StartOffset(NormalizedValue),
}

impl SamplerParam {
    /// Check if two params are the same kind (ignoring values).
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// Get the parameter display name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::SampleSelect(_) => "Sample",
            Self::PitchTracking(_) => "Pitch Track",
            Self::Level(_) => "Level",
            Self::PlayMode(_) => "Play Mode",
            Self::Direction(_) => "Direction",
            Self::VelocitySensitivity(_) => "Vel Sens",
            Self::FineTune(_) => "Fine Tune",
            Self::StartOffset(_) => "Start Offset",
        }
    }

    /// Convert to f32 for GUI sliders.
    ///
    /// Returns `None` for `SampleSelect` (not slider-compatible).
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::SampleSelect(_) => 0.0, // Not meaningful as f32
            Self::PitchTracking(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Level(g) => g.as_f32(),
            Self::PlayMode(m) => *m as u8 as f32,
            Self::Direction(d) => *d as u8 as f32,
            Self::VelocitySensitivity(v) => v.as_f32(),
            Self::FineTune(c) => c.0,
            Self::StartOffset(v) => v.as_f32(),
        }
    }

    /// Create same param type with new f32 value.
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::SampleSelect(_) => Self::SampleSelect(SampleId(value as u64)),
            Self::PitchTracking(_) => Self::PitchTracking(value > 0.5),
            Self::Level(_) => Self::Level(Gain::new(value)),
            Self::PlayMode(_) => {
                let mode = match value as u8 {
                    0 => SamplerPlayMode::OneShot,
                    1 => SamplerPlayMode::Sustain,
                    _ => SamplerPlayMode::Loop,
                };
                Self::PlayMode(mode)
            }
            Self::Direction(_) => {
                let dir = match value as u8 {
                    0 => PlayDirection::Forward,
                    1 => PlayDirection::Reverse,
                    _ => PlayDirection::PingPong,
                };
                Self::Direction(dir)
            }
            Self::VelocitySensitivity(_) => Self::VelocitySensitivity(NormalizedValue::new(value)),
            Self::FineTune(_) => Self::FineTune(Cents::new(value)),
            Self::StartOffset(_) => Self::StartOffset(NormalizedValue::new(value)),
        }
    }
}
