//! Oscillator-related parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{Cents, Gain, Hertz, NormalizedValue, Phase, Semitones};

// ============================================================================
// WAVEFORM ENUMS
// ============================================================================

/// Oscillator waveform types.
/// Note: Noise waveforms have been moved to the dedicated NoiseGenerator module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Waveform {
    #[default]
    Sine,
    Triangle,
    Sawtooth,
    Square,
    Pulse,
}

impl Waveform {
    pub const ALL: [Self; 5] = [
        Self::Sine,
        Self::Triangle,
        Self::Sawtooth,
        Self::Square,
        Self::Pulse,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Sine => "Sine",
            Self::Triangle => "Triangle",
            Self::Sawtooth => "Sawtooth",
            Self::Square => "Square",
            Self::Pulse => "Pulse",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Sine => "sine",
            Self::Triangle => "triangle",
            Self::Sawtooth => "sawtooth",
            Self::Square => "square",
            Self::Pulse => "pulse",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "sine" => Some(Self::Sine),
            "triangle" => Some(Self::Triangle),
            "sawtooth" => Some(Self::Sawtooth),
            "square" => Some(Self::Square),
            "pulse" => Some(Self::Pulse),
            // Legacy support: map noise to sine (use NoiseGenerator module instead)
            "noise" | "pink_noise" => Some(Self::Sine),
            _ => None,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|w| w == self).unwrap_or(0)
    }
}

// ============================================================================
// MATH OSCILLATOR ALGORITHM ENUM
// ============================================================================

/// Math oscillator algorithm types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum MathAlgo {
    #[default]
    SineFM,
    TanChaos,
    SuperSaw,
    BitWise,
    WaveFolder,
    Formant,
    PhaseDist,
    Metallic,
    Fractal,
    Chebyshev,
    Bytebeat,
    Lorenz,
    KarplusStrong,
    Walsh,
    Logistic,
    Pulsar,
    Shepard,
    FeedbackFM,
}

impl MathAlgo {
    pub const ALL: [Self; 18] = [
        Self::SineFM,
        Self::TanChaos,
        Self::SuperSaw,
        Self::BitWise,
        Self::WaveFolder,
        Self::Formant,
        Self::PhaseDist,
        Self::Metallic,
        Self::Fractal,
        Self::Chebyshev,
        Self::Bytebeat,
        Self::Lorenz,
        Self::KarplusStrong,
        Self::Walsh,
        Self::Logistic,
        Self::Pulsar,
        Self::Shepard,
        Self::FeedbackFM,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::SineFM => "Sine FM",
            Self::TanChaos => "Tan Chaos",
            Self::SuperSaw => "Super Saw",
            Self::BitWise => "BitWise",
            Self::WaveFolder => "Wave Folder",
            Self::Formant => "Formant",
            Self::PhaseDist => "Phase Dist",
            Self::Metallic => "Metallic",
            Self::Fractal => "Fractal",
            Self::Chebyshev => "Chebyshev",
            Self::Bytebeat => "Bytebeat",
            Self::Lorenz => "Lorenz",
            Self::KarplusStrong => "Karplus-Strong",
            Self::Walsh => "Walsh",
            Self::Logistic => "Logistic",
            Self::Pulsar => "Pulsar",
            Self::Shepard => "Shepard",
            Self::FeedbackFM => "Feedback FM",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::SineFM => "sine_fm",
            Self::TanChaos => "tan_chaos",
            Self::SuperSaw => "super_saw",
            Self::BitWise => "bitwise",
            Self::WaveFolder => "wave_folder",
            Self::Formant => "formant",
            Self::PhaseDist => "phase_dist",
            Self::Metallic => "metallic",
            Self::Fractal => "fractal",
            Self::Chebyshev => "chebyshev",
            Self::Bytebeat => "bytebeat",
            Self::Lorenz => "lorenz",
            Self::KarplusStrong => "karplus_strong",
            Self::Walsh => "walsh",
            Self::Logistic => "logistic",
            Self::Pulsar => "pulsar",
            Self::Shepard => "shepard",
            Self::FeedbackFM => "feedback_fm",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "sine_fm" => Some(Self::SineFM),
            "tan_chaos" => Some(Self::TanChaos),
            "super_saw" => Some(Self::SuperSaw),
            "bitwise" => Some(Self::BitWise),
            "wave_folder" => Some(Self::WaveFolder),
            "formant" => Some(Self::Formant),
            "phase_dist" => Some(Self::PhaseDist),
            "metallic" => Some(Self::Metallic),
            "fractal" => Some(Self::Fractal),
            "chebyshev" => Some(Self::Chebyshev),
            "bytebeat" => Some(Self::Bytebeat),
            "lorenz" => Some(Self::Lorenz),
            "karplus_strong" => Some(Self::KarplusStrong),
            "walsh" => Some(Self::Walsh),
            "logistic" => Some(Self::Logistic),
            "pulsar" => Some(Self::Pulsar),
            "shepard" => Some(Self::Shepard),
            "feedback_fm" => Some(Self::FeedbackFM),
            _ => None,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|a| a == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::modules::core::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|a| crate::modules::core::ChoiceOption::new(a.id(), a.name()))
            .collect()
    }
}

// ============================================================================
// FM MODE ENUM
// ============================================================================

/// FM mode for oscillator frequency modulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum FmMode {
    /// Exponential FM (pitch-tracking, musical intervals)
    #[default]
    Exponential,
    /// Linear FM (true frequency modulation, can go through zero)
    Linear,
}

impl FmMode {
    pub const ALL: [Self; 2] = [Self::Exponential, Self::Linear];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Exponential => "Exponential",
            Self::Linear => "Linear",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Exponential => "exponential",
            Self::Linear => "linear",
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
// OSCILLATOR PARAMETER ENUM (with typed values)
// ============================================================================

/// Oscillator parameter with typed value.
///
/// Each variant contains its properly typed value, providing compile-time
/// type safety and eliminating the need for runtime type checking.
///
/// # Example
/// ```ignore
/// use modular_synth::engine::params::OscillatorParam;
/// use modular_synth::types::Hertz;
///
/// // Type-safe parameter setting
/// let param = OscillatorParam::Frequency(Hertz::new(440.0));
///
/// // Compile error if wrong type:
/// // let wrong = OscillatorParam::Frequency(Cents::new(50.0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum OscillatorParam {
    /// Waveform selection
    Waveform(Waveform),
    /// Base frequency in Hz
    Frequency(Hertz),
    /// Fine tuning in cents (-100 to +100)
    Detune(Cents),
    /// Coarse tuning in semitones
    Octave(Semitones),
    /// Pulse width for pulse wave (0.01 to 0.99)
    PulseWidth(NormalizedValue),
    /// Output level (0.0 to 1.0)
    Level(Gain),
    /// Initial phase (0.0 to 1.0)
    Phase(Phase),
    /// FM mode (Exponential or Linear)
    FmMode(FmMode),
}

impl OscillatorParam {
    /// Check if two parameters are the same kind (ignoring values).
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// Get the parameter name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Waveform(_) => "Waveform",
            Self::Frequency(_) => "Frequency",
            Self::Detune(_) => "Detune",
            Self::Octave(_) => "Octave",
            Self::PulseWidth(_) => "Pulse Width",
            Self::Level(_) => "Level",
            Self::Phase(_) => "Phase",
            Self::FmMode(_) => "FM Mode",
        }
    }

    /// Get the value as f32 (for GUI).
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Waveform(w) => w.index() as f32,
            Self::Frequency(hz) => hz.as_f32(),
            Self::Detune(c) => c.as_f32(),
            Self::Octave(s) => s.as_f32(),
            Self::PulseWidth(pw) => pw.as_f32(),
            Self::Level(g) => g.as_f32(),
            Self::Phase(p) => p.as_f32(),
            Self::FmMode(m) => m.index() as f32,
        }
    }

    /// Create the same parameter variant with a new f32 value (for GUI).
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Waveform(_) => Self::Waveform(
                Waveform::from_index(value as usize).unwrap_or_default()
            ),
            Self::Frequency(_) => Self::Frequency(Hertz::new(value)),
            Self::Detune(_) => Self::Detune(Cents::new(value)),
            Self::Octave(_) => Self::Octave(Semitones::new(value)),
            Self::PulseWidth(_) => Self::PulseWidth(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(Gain::new(value)),
            Self::Phase(_) => Self::Phase(Phase::new(value)),
            Self::FmMode(_) => Self::FmMode(
                FmMode::from_index(value as usize).unwrap_or_default()
            ),
        }
    }

    /// Create default parameter templates (for ParameterDescriptor).
    pub fn frequency_default() -> Self { Self::Frequency(Hertz::A4) }
    pub fn waveform_default() -> Self { Self::Waveform(Waveform::default()) }
    pub fn detune_default() -> Self { Self::Detune(Cents::ZERO) }
    pub fn octave_default() -> Self { Self::Octave(Semitones::ZERO) }
    pub fn pulse_width_default() -> Self { Self::PulseWidth(NormalizedValue::CENTER) }
    pub fn level_default() -> Self { Self::Level(Gain::UNITY) }
    pub fn phase_default() -> Self { Self::Phase(Phase::ZERO) }
    pub fn fm_mode_default() -> Self { Self::FmMode(FmMode::default()) }
}

impl Default for OscillatorParam {
    fn default() -> Self {
        Self::Waveform(Waveform::default())
    }
}

// ============================================================================
// MATH OSCILLATOR PARAMETER ENUM (with typed values)
// ============================================================================

/// Math oscillator parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MathOscillatorParam {
    /// Algorithm selection
    Algorithm(MathAlgo),
    /// Base frequency in Hz
    Frequency(Hertz),
    /// Parameter A (0.0 to 1.0)
    ParamA(NormalizedValue),
    /// Parameter B (0.0 to 1.0)
    ParamB(NormalizedValue),
    /// Parameter C (0.0 to 1.0)
    ParamC(NormalizedValue),
    /// Output level (0.0 to 1.0)
    Level(Gain),
}

impl MathOscillatorParam {
    /// Check if two parameters are the same kind (ignoring values).
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// Get the parameter name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Algorithm(_) => "Algorithm",
            Self::Frequency(_) => "Frequency",
            Self::ParamA(_) => "Param A",
            Self::ParamB(_) => "Param B",
            Self::ParamC(_) => "Param C",
            Self::Level(_) => "Level",
        }
    }

    /// Get the value as f32 (for GUI).
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Algorithm(a) => a.index() as f32,
            Self::Frequency(hz) => hz.as_f32(),
            Self::ParamA(v) => v.as_f32(),
            Self::ParamB(v) => v.as_f32(),
            Self::ParamC(v) => v.as_f32(),
            Self::Level(g) => g.as_f32(),
        }
    }

    /// Create the same parameter variant with a new f32 value (for GUI).
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Algorithm(_) => Self::Algorithm(
                MathAlgo::from_index(value as usize).unwrap_or_default()
            ),
            Self::Frequency(_) => Self::Frequency(Hertz::new(value)),
            Self::ParamA(_) => Self::ParamA(NormalizedValue::new(value)),
            Self::ParamB(_) => Self::ParamB(NormalizedValue::new(value)),
            Self::ParamC(_) => Self::ParamC(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(Gain::new(value)),
        }
    }
}

impl Default for MathOscillatorParam {
    fn default() -> Self {
        Self::Algorithm(MathAlgo::default())
    }
}
