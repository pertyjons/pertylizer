//! Granular synthesis parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{Gain, Milliseconds, NormalizedValue, Seconds};

/// Grain window envelope shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GrainWindow {
    #[default]
    Hann,
    Gaussian,
    Trapezoid,
}

impl GrainWindow {
    pub const ALL: [Self; 3] = [Self::Hann, Self::Gaussian, Self::Trapezoid];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Hann => "Hann",
            Self::Gaussian => "Gaussian",
            Self::Trapezoid => "Trapezoid",
        }
    }

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Hann => "hann",
            Self::Gaussian => "gaussian",
            Self::Trapezoid => "trapezoid",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Hann => "Raised-cosine envelope — smooth grains with low artifacts.",
            Self::Gaussian => "Bell-shaped envelope — the smoothest, cleanest grains.",
            Self::Trapezoid => "Flat sustain with short ramps — louder but grittier grains.",
        }
    }

    #[must_use]
    pub fn from_index(idx: usize) -> Self {
        Self::ALL.get(idx).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|w| *w == self).unwrap_or(0)
    }
}

/// Grain source waveform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GrainSource {
    #[default]
    Saw,
    Sine,
    Square,
    Triangle,
    Noise,
}

impl GrainSource {
    pub const ALL: [Self; 5] = [
        Self::Saw,
        Self::Sine,
        Self::Square,
        Self::Triangle,
        Self::Noise,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Saw => "Saw",
            Self::Sine => "Sine",
            Self::Square => "Square",
            Self::Triangle => "Triangle",
            Self::Noise => "Noise",
        }
    }

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Saw => "saw",
            Self::Sine => "sine",
            Self::Square => "square",
            Self::Triangle => "triangle",
            Self::Noise => "noise",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Saw => "Sawtooth grains — bright, with full harmonics.",
            Self::Sine => "Sine grains — pure, smooth tone.",
            Self::Square => "Square grains — hollow, odd-harmonic character.",
            Self::Triangle => "Triangle grains — soft, mellow tone.",
            Self::Noise => "Noise grains — textured, unpitched clouds.",
        }
    }

    #[must_use]
    pub fn from_index(idx: usize) -> Self {
        Self::ALL.get(idx).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}

/// Granular oscillator parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum GranularParam {
    /// Grain size in milliseconds (5-500ms).
    GrainSize(Milliseconds),
    /// Grain density (0=sparse, 1=dense).
    Density(NormalizedValue),
    /// Playback position in source buffer (0-1).
    Position(NormalizedValue),
    /// Position randomization spread (0=fixed, 1=full range).
    PositionSpread(NormalizedValue),
    /// Pitch randomization spread in semitones (mapped 0-1 => 0-24 semitones).
    PitchSpread(NormalizedValue),
    /// Stereo pan spread (0=center, 1=full width).
    PanSpread(NormalizedValue),
    /// Freeze playback position (grains loop at current position).
    Freeze(bool),
    /// Grain window envelope shape.
    Window(GrainWindow),
    /// Source waveform.
    Source(GrainSource),
    /// Output level.
    Level(Gain),
    /// Per-oscillator glide (portamento) time in seconds (0 = follow the
    /// voice-level glide).
    GlideTime(Seconds),
}

impl Default for GranularParam {
    fn default() -> Self {
        Self::Level(Gain::UNITY)
    }
}
