//! Oscillator-related parameter types.

use serde::{Deserialize, Serialize};

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
}

// ============================================================================
// OSCILLATOR PARAMETER ENUMS
// ============================================================================

/// Oscillator parameters - only valid for oscillator modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OscillatorParam {
    /// Waveform selection (Sine, Triangle, Saw, etc.)
    Waveform,
    /// Base frequency in Hz (typically controlled by note)
    Frequency,
    /// Fine tuning in cents (-100 to +100)
    Detune,
    /// Coarse tuning in semitones
    Octave,
    /// Pulse width for pulse wave (0.0 to 1.0)
    PulseWidth,
    /// Output level (0.0 to 1.0)
    Level,
    /// Initial phase (0.0 to 1.0)
    Phase,
    /// FM mode (Exponential or Linear)
    FmMode,
}

/// Math oscillator parameters - only valid for math oscillator modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MathOscillatorParam {
    /// Algorithm selection
    Algorithm,
    /// Base frequency in Hz
    Frequency,
    /// Parameter A (0.0 to 1.0)
    ParamA,
    /// Parameter B (0.0 to 1.0)
    ParamB,
    /// Parameter C (0.0 to 1.0)
    ParamC,
    /// Output level (0.0 to 1.0)
    Level,
}
