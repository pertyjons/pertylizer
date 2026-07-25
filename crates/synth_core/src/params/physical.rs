//! Physical modeling parameter types.
//!
//! Contains parameters for physical modeling modules:
//! - KeyboardPanner: Note-based stereo positioning
//! - BodyResonance: Soundboard simulation
//! - MechanicalNoise: Key/pedal mechanical sounds

use serde::{Deserialize, Serialize};

use crate::types::{BipolarValue, Gain, Hertz, MidiNote, Milliseconds, NormalizedValue, Polarity};

// ============================================================================
// KEYBOARD PANNER PARAMETERS
// ============================================================================

/// Keyboard panner parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum KeyboardPannerParam {
    /// Pan spread amount (0.0 = mono, 1.0 = full stereo)
    Spread(NormalizedValue),
    /// Center note (MIDI note where pan is centered)
    CenterNote(MidiNote),
    /// Pan curve shape (-1.0 to 1.0, 0 = linear)
    Curve(BipolarValue),
    /// Invert panning direction (polarity)
    Invert(Polarity),
}

impl Default for KeyboardPannerParam {
    fn default() -> Self {
        Self::Spread(NormalizedValue::new(0.5))
    }
}

// ============================================================================
// SPATIAL PANNER PARAMETERS
// ============================================================================

/// Spatial panner parameter with typed value.
///
/// Positions a mono voice in a fixed virtual room via ISM early reflections
/// and a binaural (ITD/ILD) direct path. `x`/`y`/`z` are bipolar offsets from
/// the room centre (listener position).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SpatialPannerParam {
    /// Left/right source offset (-1 = full left, +1 = full right).
    X(BipolarValue),
    /// Front/back source offset (-1 = behind, +1 = in front).
    Y(BipolarValue),
    /// Down/up source offset (-1 = below, +1 = above).
    Z(BipolarValue),
    /// Material diffusion / scattering of the reflections (0..1).
    Diffusion(NormalizedValue),
    /// Early-reflection (room) level (0..1).
    ErLevel(NormalizedValue),
    /// Direct (binaural) path level (0..1).
    DirectLevel(NormalizedValue),
    /// Wall absorption amount (0 = reflective, 1 = dead) (0..1).
    Absorption(NormalizedValue),
    /// Air absorption (high-frequency damping over distance) (0..1).
    AirAbsorption(NormalizedValue),
    /// Radial distance scale on the `x`/`y`/`z` offset vector (0 = at the
    /// listener, 1 = the full offset). Drives Doppler, inverse-distance level,
    /// and air-absorption rolloff together (0..1).
    Distance(NormalizedValue),
}

impl Default for SpatialPannerParam {
    fn default() -> Self {
        Self::X(BipolarValue::CENTER)
    }
}

// ============================================================================
// BODY RESONANCE PARAMETERS
// ============================================================================

/// Body resonance parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BodyResonanceParam {
    /// Primary resonant frequency
    Frequency(Hertz),
    /// Resonance amount (Q factor)
    Resonance(NormalizedValue),
    /// Body size simulation (affects frequency spread)
    Size(NormalizedValue),
    /// Wood type simulation (brightness)
    Brightness(NormalizedValue),
    /// Wet/dry mix
    Mix(NormalizedValue),
}

impl Default for BodyResonanceParam {
    fn default() -> Self {
        Self::Frequency(Hertz::new(200.0))
    }
}

// ============================================================================
// MECHANICAL NOISE PARAMETERS
// ============================================================================

/// Mechanical noise type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MechanicalNoiseType {
    /// Key down noise
    #[default]
    KeyDown,
    /// Key release noise
    KeyUp,
    /// Damper pedal noise
    Pedal,
    /// Hammer strike
    Hammer,
}

impl MechanicalNoiseType {
    pub const ALL: [Self; 4] = [Self::KeyDown, Self::KeyUp, Self::Pedal, Self::Hammer];

    pub fn name(&self) -> &'static str {
        match self {
            Self::KeyDown => "Key Down",
            Self::KeyUp => "Key Up",
            Self::Pedal => "Pedal",
            Self::Hammer => "Hammer",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::KeyDown => "Key-press mechanical noise — the thud as a key is pressed.",
            Self::KeyUp => "Key-release mechanical noise — the click as a key returns.",
            Self::Pedal => "Damper-pedal mechanism noise.",
            Self::Hammer => "Hammer-strike noise against the string.",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::KeyDown => "key_down",
            Self::KeyUp => "key_up",
            Self::Pedal => "pedal",
            Self::Hammer => "hammer",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|t| t == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|t| {
                crate::module_traits::ChoiceOption::new(t.id(), t.name())
                    .with_description(t.description())
            })
            .collect()
    }
}

/// Mechanical noise parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MechanicalNoiseParam {
    /// Noise type
    NoiseType(MechanicalNoiseType),
    /// Duration of the noise burst
    Duration(Milliseconds),
    /// Filter cutoff for noise shaping
    Cutoff(Hertz),
    /// Velocity sensitivity (0.0 to 1.0)
    VelocitySens(NormalizedValue),
    /// Output level
    Level(Gain),
}

impl Default for MechanicalNoiseParam {
    fn default() -> Self {
        Self::NoiseType(MechanicalNoiseType::default())
    }
}
