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
            Self::CenterNote(_) => Self::CenterNote(MidiNote::new(value.clamp(0.0, 127.0) as u8)),
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
            .map(|t| crate::module_traits::ChoiceOption::new(t.id(), t.name()))
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
