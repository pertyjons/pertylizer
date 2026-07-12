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

impl KeyboardPannerParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Spread(_) => "Spread",
            Self::CenterNote(_) => "Center",
            Self::Curve(_) => "Curve",
            Self::Invert(_) => "Invert",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Spread(v) => v.as_f32(),
            Self::CenterNote(n) => f32::from(n.as_u8()),
            Self::Curve(c) => c.as_f32(),
            Self::Invert(p) => {
                if p.is_inverted() {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Spread(_) => Self::Spread(NormalizedValue::new(value)),
            Self::CenterNote(_) => {
                Self::CenterNote(MidiNote::new(value.round().clamp(0.0, 127.0) as u8))
            }
            Self::Curve(_) => Self::Curve(BipolarValue::new(value)),
            Self::Invert(_) => Self::Invert(Polarity::from(value > 0.5)),
        }
    }
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
}

impl SpatialPannerParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::X(_) => "X",
            Self::Y(_) => "Y",
            Self::Z(_) => "Z",
            Self::Diffusion(_) => "Diffusion",
            Self::ErLevel(_) => "ER Level",
            Self::DirectLevel(_) => "Direct Level",
            Self::Absorption(_) => "Absorption",
            Self::AirAbsorption(_) => "Air Absorption",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::X(v) | Self::Y(v) | Self::Z(v) => v.as_f32(),
            Self::Diffusion(v)
            | Self::ErLevel(v)
            | Self::DirectLevel(v)
            | Self::Absorption(v)
            | Self::AirAbsorption(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::X(_) => Self::X(BipolarValue::new(value)),
            Self::Y(_) => Self::Y(BipolarValue::new(value)),
            Self::Z(_) => Self::Z(BipolarValue::new(value)),
            Self::Diffusion(_) => Self::Diffusion(NormalizedValue::new(value)),
            Self::ErLevel(_) => Self::ErLevel(NormalizedValue::new(value)),
            Self::DirectLevel(_) => Self::DirectLevel(NormalizedValue::new(value)),
            Self::Absorption(_) => Self::Absorption(NormalizedValue::new(value)),
            Self::AirAbsorption(_) => Self::AirAbsorption(NormalizedValue::new(value)),
        }
    }
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

impl BodyResonanceParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Frequency(_) => "Freq",
            Self::Resonance(_) => "Resonance",
            Self::Size(_) => "Size",
            Self::Brightness(_) => "Bright",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Frequency(f) => f.as_f32(),
            Self::Resonance(v) | Self::Size(v) | Self::Brightness(v) | Self::Mix(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Frequency(_) => Self::Frequency(Hertz::new(value.clamp(50.0, 2000.0))),
            Self::Resonance(_) => Self::Resonance(NormalizedValue::new(value)),
            Self::Size(_) => Self::Size(NormalizedValue::new(value)),
            Self::Brightness(_) => Self::Brightness(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
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

impl MechanicalNoiseParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::NoiseType(_) => "Type",
            Self::Duration(_) => "Duration",
            Self::Cutoff(_) => "Cutoff",
            Self::VelocitySens(_) => "Vel Sens",
            Self::Level(_) => "Level",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::NoiseType(t) => t.index() as f32,
            Self::Duration(d) => d.as_f32(),
            Self::Cutoff(f) => f.as_f32(),
            Self::VelocitySens(v) => v.as_f32(),
            Self::Level(g) => g.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::NoiseType(_) => {
                Self::NoiseType(MechanicalNoiseType::from_index(value as usize).unwrap_or_default())
            }
            Self::Duration(_) => Self::Duration(Milliseconds::new(value.clamp(1.0, 100.0))),
            Self::Cutoff(_) => Self::Cutoff(Hertz::new(value.clamp(100.0, 10000.0))),
            Self::VelocitySens(_) => Self::VelocitySens(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(Gain::new(value)),
        }
    }
}

impl Default for MechanicalNoiseParam {
    fn default() -> Self {
        Self::NoiseType(MechanicalNoiseType::default())
    }
}
