//! Physical modeling parameter types.
//!
//! Contains parameters for all physical modeling modules:
//! - StringResonator: Karplus-Strong string synthesis
//! - ResonatorBank: Sympathetic resonance modeling
//! - KeyboardPanner: Note-based stereo positioning
//! - BodyResonance: Soundboard simulation
//! - VelocityMapper: Velocity curve shaping
//! - MechanicalNoise: Key/pedal mechanical sounds

use serde::{Deserialize, Serialize};

use crate::types::{Gain, Hertz, Milliseconds, NormalizedValue};

// ============================================================================
// STRING RESONATOR PARAMETERS
// ============================================================================

/// String resonator parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum StringResonatorParam {
    /// Feedback amount (0.0 to 1.0) - controls decay time
    Feedback(NormalizedValue),
    /// Damping (0.0 to 1.0) - higher = darker sound
    Damping(NormalizedValue),
    /// Inharmonicity (0.0 to 1.0) - stiffness of string
    Inharmonicity(NormalizedValue),
    /// Brightness (0.0 to 1.0) - initial excitation brightness
    Brightness(NormalizedValue),
    /// Output level
    Level(Gain),
}

impl StringResonatorParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Feedback(_) => "Feedback",
            Self::Damping(_) => "Damping",
            Self::Inharmonicity(_) => "Inharm",
            Self::Brightness(_) => "Bright",
            Self::Level(_) => "Level",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Feedback(v) | Self::Damping(v) | Self::Inharmonicity(v) | Self::Brightness(v) => {
                v.as_f32()
            }
            Self::Level(g) => g.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Feedback(_) => Self::Feedback(NormalizedValue::new(value)),
            Self::Damping(_) => Self::Damping(NormalizedValue::new(value)),
            Self::Inharmonicity(_) => Self::Inharmonicity(NormalizedValue::new(value)),
            Self::Brightness(_) => Self::Brightness(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(Gain::new(value)),
        }
    }
}

impl Default for StringResonatorParam {
    fn default() -> Self {
        Self::Feedback(NormalizedValue::new(0.99))
    }
}

// ============================================================================
// RESONATOR BANK PARAMETERS
// ============================================================================

/// Resonator bank parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ResonatorBankParam {
    /// Number of resonator strings (1-12)
    StringCount(u8),
    /// Sympathetic resonance amount
    Resonance(NormalizedValue),
    /// Detuning between strings (cents)
    Detune(f32),
    /// Decay time multiplier
    DecayMult(NormalizedValue),
    /// Output level
    Level(Gain),
}

impl ResonatorBankParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::StringCount(_) => "Strings",
            Self::Resonance(_) => "Resonance",
            Self::Detune(_) => "Detune",
            Self::DecayMult(_) => "Decay",
            Self::Level(_) => "Level",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::StringCount(n) => f32::from(*n),
            Self::Resonance(v) | Self::DecayMult(v) => v.as_f32(),
            Self::Detune(c) => *c,
            Self::Level(g) => g.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::StringCount(_) => Self::StringCount(value.clamp(1.0, 12.0) as u8),
            Self::Resonance(_) => Self::Resonance(NormalizedValue::new(value)),
            Self::Detune(_) => Self::Detune(value.clamp(-50.0, 50.0)),
            Self::DecayMult(_) => Self::DecayMult(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(Gain::new(value)),
        }
    }
}

impl Default for ResonatorBankParam {
    fn default() -> Self {
        Self::StringCount(3)
    }
}

// ============================================================================
// KEYBOARD PANNER PARAMETERS
// ============================================================================

/// Keyboard panner parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum KeyboardPannerParam {
    /// Pan spread amount (0.0 = mono, 1.0 = full stereo)
    Spread(NormalizedValue),
    /// Center note (MIDI note number where pan is centered)
    CenterNote(u8),
    /// Pan curve shape (-1.0 to 1.0, 0 = linear)
    Curve(f32),
    /// Invert panning direction
    Invert(bool),
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
            Self::CenterNote(n) => f32::from(*n),
            Self::Curve(c) => *c,
            Self::Invert(b) => {
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
            Self::Spread(_) => Self::Spread(NormalizedValue::new(value)),
            Self::CenterNote(_) => Self::CenterNote(value.clamp(0.0, 127.0) as u8),
            Self::Curve(_) => Self::Curve(value.clamp(-1.0, 1.0)),
            Self::Invert(_) => Self::Invert(value > 0.5),
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
// VELOCITY MAPPER PARAMETERS
// ============================================================================

/// Velocity curve type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum VelocityCurve {
    /// Linear response
    #[default]
    Linear,
    /// Soft touch - more response at low velocities
    Soft,
    /// Hard touch - more response at high velocities
    Hard,
    /// S-Curve - smooth response
    SCurve,
    /// Fixed - ignore velocity
    Fixed,
}

impl VelocityCurve {
    pub const ALL: [Self; 5] = [
        Self::Linear,
        Self::Soft,
        Self::Hard,
        Self::SCurve,
        Self::Fixed,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Soft => "Soft",
            Self::Hard => "Hard",
            Self::SCurve => "S-Curve",
            Self::Fixed => "Fixed",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Soft => "soft",
            Self::Hard => "hard",
            Self::SCurve => "s_curve",
            Self::Fixed => "fixed",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|t| t == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::modules::core::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|t| crate::modules::core::ChoiceOption::new(t.id(), t.name()))
            .collect()
    }
}

/// Velocity mapper parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum VelocityMapperParam {
    /// Velocity curve type
    Curve(VelocityCurve),
    /// Minimum output velocity
    MinOut(NormalizedValue),
    /// Maximum output velocity
    MaxOut(NormalizedValue),
    /// Curve amount (-1.0 to 1.0)
    CurveAmount(f32),
}

impl VelocityMapperParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Curve(_) => "Curve",
            Self::MinOut(_) => "Min",
            Self::MaxOut(_) => "Max",
            Self::CurveAmount(_) => "Amount",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Curve(c) => c.index() as f32,
            Self::MinOut(v) | Self::MaxOut(v) => v.as_f32(),
            Self::CurveAmount(a) => *a,
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Curve(_) => Self::Curve(VelocityCurve::from_index(value as usize).unwrap_or_default()),
            Self::MinOut(_) => Self::MinOut(NormalizedValue::new(value)),
            Self::MaxOut(_) => Self::MaxOut(NormalizedValue::new(value)),
            Self::CurveAmount(_) => Self::CurveAmount(value.clamp(-1.0, 1.0)),
        }
    }
}

impl Default for VelocityMapperParam {
    fn default() -> Self {
        Self::Curve(VelocityCurve::default())
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

    pub fn to_choices() -> Vec<crate::modules::core::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|t| crate::modules::core::ChoiceOption::new(t.id(), t.name()))
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
