//! Oscillator-related parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{
    BipolarValue, Cents, Gain, Hertz, NormalizedValue, Phase, PulseWidth, Semitones,
};

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
    DsfSaw,
}

impl Waveform {
    pub const ALL: [Self; 6] = [
        Self::Sine,
        Self::Triangle,
        Self::Sawtooth,
        Self::Square,
        Self::Pulse,
        Self::DsfSaw,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Sine => "Sine",
            Self::Triangle => "Triangle",
            Self::Sawtooth => "Sawtooth",
            Self::Square => "Square",
            Self::Pulse => "Pulse",
            Self::DsfSaw => "DSF Saw",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Sine => "sine",
            Self::Triangle => "triangle",
            Self::Sawtooth => "sawtooth",
            Self::Square => "square",
            Self::Pulse => "pulse",
            Self::DsfSaw => "dsf_saw",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "sine" => Some(Self::Sine),
            "triangle" => Some(Self::Triangle),
            "sawtooth" => Some(Self::Sawtooth),
            "square" => Some(Self::Square),
            "pulse" => Some(Self::Pulse),
            "dsf_saw" => Some(Self::DsfSaw),
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
    Vosim,
}

impl MathAlgo {
    pub const ALL: [Self; 19] = [
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
        Self::Vosim,
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
            Self::Vosim => "VOSIM",
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
            Self::Vosim => "vosim",
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
            "vosim" => Some(Self::Vosim),
            _ => None,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|a| a == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|a| crate::module_traits::ChoiceOption::new(a.id(), a.name()))
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

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| crate::module_traits::ChoiceOption::new(m.id(), m.name()))
            .collect()
    }
}

// ============================================================================
// ANTI-ALIAS MODE ENUM
// ============================================================================

/// Anti-aliasing mode for the oscillator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AntiAliasMode {
    /// Polynomial band-limited step (lightweight, good quality)
    #[default]
    PolyBlep,
    /// Minimum-phase band-limited step (higher quality, uses lookup table)
    MinBlep,
}

impl AntiAliasMode {
    pub const ALL: [Self; 2] = [Self::PolyBlep, Self::MinBlep];

    pub fn name(self) -> &'static str {
        match self {
            Self::PolyBlep => "PolyBLEP",
            Self::MinBlep => "MinBLEP",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::PolyBlep => "polyblep",
            Self::MinBlep => "minblep",
        }
    }

    pub fn from_index(idx: usize) -> Self {
        match idx {
            1 => Self::MinBlep,
            _ => Self::PolyBlep,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::PolyBlep => 0,
            Self::MinBlep => 1,
        }
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
/// use pertylizer::engine::params::OscillatorParam;
/// use pertylizer::types::Hertz;
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
    PulseWidth(PulseWidth),
    /// Output level (0.0 to 1.0)
    Level(Gain),
    /// Initial phase (0.0 to 1.0)
    Phase(Phase),
    /// FM mode (Exponential or Linear)
    FmMode(FmMode),
    /// FM input attenuverter (-1.0 to 1.0)
    FmAmount(BipolarValue),
    /// Number of unison voices (1-7, 1 = off)
    UnisonVoices(u8),
    /// Total unison detune spread in cents (0-100)
    UnisonDetune(Cents),
    /// Unison stereo spread (0.0 = mono, 1.0 = full)
    UnisonSpread(NormalizedValue),
    /// Phase randomization on note-on (0.0 = none, 1.0 = full)
    UnisonPhaseRandom(NormalizedValue),
    /// Cross-modulation amount from another oscillator (0.0 = off, 1.0 = full)
    CrossModAmount(NormalizedValue),
    /// Anti-aliasing algorithm
    AntiAlias(AntiAliasMode),
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
            Self::FmAmount(_) => "FM Amount",
            Self::UnisonVoices(_) => "Unison",
            Self::UnisonDetune(_) => "Uni Detune",
            Self::UnisonSpread(_) => "Uni Spread",
            Self::UnisonPhaseRandom(_) => "Uni Phase",
            Self::CrossModAmount(_) => "X-Mod",
            Self::AntiAlias(_) => "Anti-Alias",
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
            Self::FmAmount(a) => a.as_f32(),
            Self::UnisonVoices(n) => f32::from(*n),
            Self::UnisonDetune(c) => c.as_f32(),
            Self::UnisonSpread(v) => v.as_f32(),
            Self::UnisonPhaseRandom(v) => v.as_f32(),
            Self::CrossModAmount(v) => v.as_f32(),
            #[allow(clippy::cast_precision_loss)]
            Self::AntiAlias(m) => m.index() as f32,
        }
    }

    /// Create the same parameter variant with a new f32 value (for GUI).
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Waveform(_) => {
                Self::Waveform(Waveform::from_index(value as usize).unwrap_or_default())
            }
            Self::Frequency(_) => Self::Frequency(Hertz::new(value)),
            // Clamp to the descriptor range (-100..100 cents); mirrors the
            // `UnisonDetune` clamp below so every apply path stays in range.
            Self::Detune(_) => Self::Detune(Cents::new(value.clamp(-100.0, 100.0))),
            Self::Octave(_) => Self::Octave(Semitones::new(value)),
            Self::PulseWidth(_) => Self::PulseWidth(PulseWidth::new(value)),
            Self::Level(_) => Self::Level(Gain::new(value)),
            Self::Phase(_) => Self::Phase(Phase::new(value)),
            Self::FmMode(_) => Self::FmMode(FmMode::from_index(value as usize).unwrap_or_default()),
            Self::FmAmount(_) => Self::FmAmount(BipolarValue::new(value)),
            #[allow(clippy::cast_possible_truncation)]
            Self::UnisonVoices(_) => Self::UnisonVoices((value.round() as u8).clamp(1, 7)),
            Self::UnisonDetune(_) => Self::UnisonDetune(Cents::new(value.clamp(0.0, 100.0))),
            Self::UnisonSpread(_) => Self::UnisonSpread(NormalizedValue::new(value)),
            Self::UnisonPhaseRandom(_) => Self::UnisonPhaseRandom(NormalizedValue::new(value)),
            Self::CrossModAmount(_) => Self::CrossModAmount(NormalizedValue::new(value)),
            #[allow(clippy::cast_possible_truncation)]
            Self::AntiAlias(_) => Self::AntiAlias(AntiAliasMode::from_index(value as usize)),
        }
    }

    /// Create default parameter templates (for ParameterDescriptor).
    pub fn frequency_default() -> Self {
        Self::Frequency(Hertz::A4)
    }
    pub fn waveform_default() -> Self {
        Self::Waveform(Waveform::default())
    }
    pub fn detune_default() -> Self {
        Self::Detune(Cents::ZERO)
    }
    pub fn octave_default() -> Self {
        Self::Octave(Semitones::ZERO)
    }
    pub fn pulse_width_default() -> Self {
        Self::PulseWidth(PulseWidth::SQUARE)
    }
    pub fn level_default() -> Self {
        Self::Level(Gain::UNITY)
    }
    pub fn phase_default() -> Self {
        Self::Phase(Phase::ZERO)
    }
    pub fn fm_mode_default() -> Self {
        Self::FmMode(FmMode::default())
    }
    pub fn fm_amount_default() -> Self {
        Self::FmAmount(BipolarValue::MAX)
    }
    pub fn unison_voices_default() -> Self {
        Self::UnisonVoices(1)
    }
    pub fn unison_detune_default() -> Self {
        Self::UnisonDetune(Cents::new(10.0))
    }
    pub fn unison_spread_default() -> Self {
        Self::UnisonSpread(NormalizedValue::CENTER)
    }
    pub fn unison_phase_random_default() -> Self {
        Self::UnisonPhaseRandom(NormalizedValue::MAX)
    }
    pub fn cross_mod_amount_default() -> Self {
        Self::CrossModAmount(NormalizedValue::MIN)
    }
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
            Self::Algorithm(_) => {
                Self::Algorithm(MathAlgo::from_index(value as usize).unwrap_or_default())
            }
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
