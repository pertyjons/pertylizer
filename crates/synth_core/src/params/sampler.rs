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
pub struct SampleId(u64);

impl SampleId {
    #[must_use]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

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
    /// Ergonomic constructor for `SampleSelect` from a raw sample id.
    ///
    /// Saves call sites from spelling out `SampleSelect(SampleId(id))`.
    #[must_use]
    pub const fn sample_select(id: u64) -> Self {
        Self::SampleSelect(SampleId::new(id))
    }
}
