//! Type-safe parameter system for the modular synthesizer.
//!
//! This module provides strongly-typed parameters that prevent
//! common errors like sending the wrong parameter to the wrong module.
//!
//! # Design Philosophy
//!
//! Instead of a flat `ParameterId` enum, we use nested enums that
//! encode both the module type and the parameter in the type system.
//!
//! This means the compiler will catch errors like:
//! - Sending `OscillatorParam::Waveform` to a Filter module
//! - Using a string "sine" instead of `Waveform::Sine`

use serde::{Deserialize, Serialize};

// ============================================================================
// MODULE TYPE ENUM
// ============================================================================

/// Strongly-typed module type identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModuleType {
    Oscillator,
    MathOscillator,
    Filter,
    Envelope,
    Lfo,
    Amplifier,
    Mixer,
    StereoOutput,
    // Effects
    Delay,
    Reverb,
    Distortion,
    Chorus,
    Phaser,
    Flanger,
    Compressor,
    Eq,
    // Sample-based
    SamplePlayer,
    SampleRecorder,
    GranularSynth,
    Wavetable,
    // Visualizers
    Oscilloscope,
    LevelMeter,
}

impl ModuleType {
    /// Get the display name for this module type.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Oscillator => "Oscillator",
            Self::MathOscillator => "Math Oscillator",
            Self::Filter => "Filter",
            Self::Envelope => "Envelope",
            Self::Lfo => "LFO",
            Self::Amplifier => "Amplifier",
            Self::Mixer => "Mixer",
            Self::StereoOutput => "Stereo Output",
            Self::Delay => "Delay",
            Self::Reverb => "Reverb",
            Self::Distortion => "Distortion",
            Self::Chorus => "Chorus",
            Self::Phaser => "Phaser",
            Self::Flanger => "Flanger",
            Self::Compressor => "Compressor",
            Self::Eq => "EQ",
            Self::SamplePlayer => "Sample Player",
            Self::SampleRecorder => "Sample Recorder",
            Self::GranularSynth => "Granular",
            Self::Wavetable => "Wavetable",
            Self::Oscilloscope => "Oscilloscope",
            Self::LevelMeter => "Level Meter",
        }
    }

    /// Get a short 3-letter prefix for module IDs.
    pub fn prefix(&self) -> &'static str {
        match self {
            Self::Oscillator => "osc",
            Self::MathOscillator => "mth",
            Self::Filter => "flt",
            Self::Envelope => "env",
            Self::Lfo => "lfo",
            Self::Amplifier => "amp",
            Self::Mixer => "mix",
            Self::StereoOutput => "out",
            Self::Delay => "dly",
            Self::Reverb => "rev",
            Self::Distortion => "dst",
            Self::Chorus => "chr",
            Self::Phaser => "phs",
            Self::Flanger => "fln",
            Self::Compressor => "cmp",
            Self::Eq => "equ",
            Self::SamplePlayer => "spl",
            Self::SampleRecorder => "rec",
            Self::GranularSynth => "grn",
            Self::Wavetable => "wtb",
            Self::Oscilloscope => "scp",
            Self::LevelMeter => "mtr",
        }
    }

    /// Parse a prefix string back to ModuleType.
    pub fn from_prefix(prefix: &str) -> Option<Self> {
        match prefix {
            "osc" => Some(Self::Oscillator),
            "mth" => Some(Self::MathOscillator),
            "flt" => Some(Self::Filter),
            "env" => Some(Self::Envelope),
            "lfo" => Some(Self::Lfo),
            "amp" => Some(Self::Amplifier),
            "mix" => Some(Self::Mixer),
            "out" => Some(Self::StereoOutput),
            "dly" => Some(Self::Delay),
            "rev" => Some(Self::Reverb),
            "dst" => Some(Self::Distortion),
            "chr" => Some(Self::Chorus),
            "phs" => Some(Self::Phaser),
            "fln" => Some(Self::Flanger),
            "cmp" => Some(Self::Compressor),
            "equ" => Some(Self::Eq),
            "spl" => Some(Self::SamplePlayer),
            "rec" => Some(Self::SampleRecorder),
            "grn" => Some(Self::GranularSynth),
            "wtb" => Some(Self::Wavetable),
            "scp" => Some(Self::Oscilloscope),
            "mtr" => Some(Self::LevelMeter),
            _ => None,
        }
    }
    
    /// Convert to patch::ModuleType for storage.
    pub fn to_module_type(self) -> crate::patch::ModuleType {
        use crate::patch::ModuleType as PM;
        match self {
            Self::Oscillator => PM::Oscillator,
            Self::MathOscillator => PM::MathOscillator,
            Self::Filter => PM::Filter,
            Self::Envelope => PM::Envelope,
            Self::Lfo => PM::Lfo,
            Self::Amplifier => PM::Amplifier,
            Self::Mixer => PM::Mixer,
            Self::StereoOutput => PM::StereoOutput,
            Self::Delay => PM::Delay,
            Self::Reverb => PM::Reverb,
            Self::Distortion => PM::Distortion,
            Self::Chorus => PM::Chorus,
            Self::Oscilloscope => PM::Oscilloscope,
            Self::LevelMeter => PM::LevelMeter,
            // These don't have patch equivalents yet, default to Mixer
            Self::Phaser | Self::Flanger | Self::Compressor | Self::Eq |
            Self::SamplePlayer | Self::SampleRecorder | Self::GranularSynth | Self::Wavetable => PM::Mixer,
        }
    }
}

// ============================================================================
// WAVEFORM ENUMS (replacing string-based choices)
// ============================================================================

/// Oscillator waveform types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Waveform {
    #[default]
    Sine,
    Triangle,
    Sawtooth,
    Square,
    Pulse,
    Noise,
    PinkNoise,
}

impl Waveform {
    pub const ALL: [Self; 7] = [
        Self::Sine,
        Self::Triangle,
        Self::Sawtooth,
        Self::Square,
        Self::Pulse,
        Self::Noise,
        Self::PinkNoise,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Sine => "Sine",
            Self::Triangle => "Triangle",
            Self::Sawtooth => "Sawtooth",
            Self::Square => "Square",
            Self::Pulse => "Pulse",
            Self::Noise => "White Noise",
            Self::PinkNoise => "Pink Noise",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Sine => "sine",
            Self::Triangle => "triangle",
            Self::Sawtooth => "sawtooth",
            Self::Square => "square",
            Self::Pulse => "pulse",
            Self::Noise => "noise",
            Self::PinkNoise => "pink_noise",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "sine" => Some(Self::Sine),
            "triangle" => Some(Self::Triangle),
            "sawtooth" => Some(Self::Sawtooth),
            "square" => Some(Self::Square),
            "pulse" => Some(Self::Pulse),
            "noise" => Some(Self::Noise),
            "pink_noise" => Some(Self::PinkNoise),
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

/// LFO waveform types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum LfoWaveform {
    #[default]
    Sine,
    Triangle,
    Sawtooth,
    Square,
    SampleAndHold,
}

impl LfoWaveform {
    pub const ALL: [Self; 5] = [
        Self::Sine,
        Self::Triangle,
        Self::Sawtooth,
        Self::Square,
        Self::SampleAndHold,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Sine => "Sine",
            Self::Triangle => "Triangle",
            Self::Sawtooth => "Sawtooth",
            Self::Square => "Square",
            Self::SampleAndHold => "S&H",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Sine => "sine",
            Self::Triangle => "triangle",
            Self::Sawtooth => "sawtooth",
            Self::Square => "square",
            Self::SampleAndHold => "sample_and_hold",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "sine" => Some(Self::Sine),
            "triangle" => Some(Self::Triangle),
            "sawtooth" => Some(Self::Sawtooth),
            "square" => Some(Self::Square),
            "sample_and_hold" | "s&h" => Some(Self::SampleAndHold),
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

/// Filter types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum FilterMode {
    #[default]
    Lowpass,
    Highpass,
    Bandpass,
    Notch,
    Peak,
    LowShelf,
    HighShelf,
}

impl FilterMode {
    pub const ALL: [Self; 7] = [
        Self::Lowpass,
        Self::Highpass,
        Self::Bandpass,
        Self::Notch,
        Self::Peak,
        Self::LowShelf,
        Self::HighShelf,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Lowpass => "Lowpass",
            Self::Highpass => "Highpass",
            Self::Bandpass => "Bandpass",
            Self::Notch => "Notch",
            Self::Peak => "Peak",
            Self::LowShelf => "Low Shelf",
            Self::HighShelf => "High Shelf",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Lowpass => "lowpass",
            Self::Highpass => "highpass",
            Self::Bandpass => "bandpass",
            Self::Notch => "notch",
            Self::Peak => "peak",
            Self::LowShelf => "low_shelf",
            Self::HighShelf => "high_shelf",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "lowpass" | "lp" => Some(Self::Lowpass),
            "highpass" | "hp" => Some(Self::Highpass),
            "bandpass" | "bp" => Some(Self::Bandpass),
            "notch" => Some(Self::Notch),
            "peak" => Some(Self::Peak),
            "low_shelf" => Some(Self::LowShelf),
            "high_shelf" => Some(Self::HighShelf),
            _ => None,
        }
    }
    
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
    
    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }
}

/// Delay modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum DelayMode {
    #[default]
    Mono,
    Stereo,
    PingPong,
}

impl DelayMode {
    pub const ALL: [Self; 3] = [Self::Mono, Self::Stereo, Self::PingPong];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Mono => "Mono",
            Self::Stereo => "Stereo",
            Self::PingPong => "Ping-Pong",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Mono => "mono",
            Self::Stereo => "stereo",
            Self::PingPong => "ping_pong",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "mono" => Some(Self::Mono),
            "stereo" => Some(Self::Stereo),
            "ping_pong" => Some(Self::PingPong),
            _ => None,
        }
    }
    
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
    
    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }
}

/// Distortion types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum DistortionMode {
    #[default]
    SoftClip,
    HardClip,
    Tube,
    Foldback,
    Bitcrush,
}

impl DistortionMode {
    pub const ALL: [Self; 5] = [
        Self::SoftClip,
        Self::HardClip,
        Self::Tube,
        Self::Foldback,
        Self::Bitcrush,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::SoftClip => "Soft Clip",
            Self::HardClip => "Hard Clip",
            Self::Tube => "Tube",
            Self::Foldback => "Foldback",
            Self::Bitcrush => "Bitcrush",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::SoftClip => "soft_clip",
            Self::HardClip => "hard_clip",
            Self::Tube => "tube",
            Self::Foldback => "foldback",
            Self::Bitcrush => "bitcrush",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "soft_clip" => Some(Self::SoftClip),
            "hard_clip" => Some(Self::HardClip),
            "tube" => Some(Self::Tube),
            "foldback" => Some(Self::Foldback),
            "bitcrush" => Some(Self::Bitcrush),
            _ => None,
        }
    }
    
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
    
    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }
}

/// Loop modes for sample playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum LoopMode {
    #[default]
    Off,
    Forward,
    Backward,
    PingPong,
}

impl LoopMode {
    pub const ALL: [Self; 4] = [Self::Off, Self::Forward, Self::Backward, Self::PingPong];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Forward => "Forward",
            Self::Backward => "Backward",
            Self::PingPong => "Ping-Pong",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Forward => "forward",
            Self::Backward => "backward",
            Self::PingPong => "ping_pong",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "off" => Some(Self::Off),
            "forward" => Some(Self::Forward),
            "backward" => Some(Self::Backward),
            "ping_pong" => Some(Self::PingPong),
            _ => None,
        }
    }
    
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
    
    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
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
// MODULE-SPECIFIC PARAMETER ENUMS
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

/// Filter parameters - only valid for filter modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FilterParam {
    /// Filter mode (Lowpass, Highpass, etc.)
    Mode,
    /// Cutoff frequency in Hz
    Cutoff,
    /// Resonance/Q (0.0 to 1.0, or actual Q value)
    Resonance,
    /// Keyboard tracking amount (0.0 to 1.0)
    KeyTracking,
    /// Drive/saturation amount
    Drive,
    /// Envelope amount (-1.0 to 1.0, scales envelope CV input)
    EnvAmount,
}

/// Envelope (ADSR) parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnvelopeParam {
    /// Attack time in seconds
    Attack,
    /// Decay time in seconds
    Decay,
    /// Sustain level (0.0 to 1.0)
    Sustain,
    /// Release time in seconds
    Release,
    /// Attack curve shape (-1.0 to 1.0, 0 = linear)
    AttackCurve,
    /// Decay curve shape
    DecayCurve,
    /// Release curve shape
    ReleaseCurve,
    /// Velocity sensitivity (0.0 = none, 1.0 = full)
    VelocitySensitivity,
}

/// LFO parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LfoParam {
    /// LFO waveform
    Waveform,
    /// Rate in Hz
    Rate,
    /// Modulation depth (0.0 to 1.0)
    Depth,
    /// Initial phase (0.0 to 1.0)
    Phase,
    /// Sync to tempo
    TempoSync,
    /// Retrigger on note
    Retrigger,
}

/// Amplifier/VCA parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AmplifierParam {
    /// Output level (0.0 to 1.0)
    Level,
    /// Stereo pan (-1.0 to 1.0)
    Pan,
}

/// Mixer parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MixerParam {
    /// Input 1 level
    Input1,
    /// Input 2 level
    Input2,
    /// Input 3 level
    Input3,
    /// Input 4 level
    Input4,
    /// Master output level
    Master,
    /// Mute toggle
    Mute,
    /// Limiter enable toggle
    Limit,
}

/// Delay effect parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DelayParam {
    /// Delay mode (Mono, Stereo, PingPong)
    Mode,
    /// Delay time in seconds
    Time,
    /// Left channel delay time (for stereo mode)
    TimeLeft,
    /// Right channel delay time (for stereo mode)
    TimeRight,
    /// Feedback amount (0.0 to 1.0)
    Feedback,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
    /// High-frequency damping
    Damping,
    /// Tempo sync
    TempoSync,
}

/// Reverb effect parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReverbParam {
    /// Room size (0.0 to 1.0)
    RoomSize,
    /// Pre-delay time in seconds
    PreDelay,
    /// High-frequency damping (0.0 to 1.0)
    Damping,
    /// Stereo width (0.0 to 1.0)
    Width,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
}

/// Distortion effect parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DistortionParam {
    /// Distortion type/mode
    Mode,
    /// Drive amount (0.0 to 1.0)
    Drive,
    /// Tone/brightness (0.0 to 1.0)
    Tone,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
}

/// Chorus effect parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChorusParam {
    /// Modulation rate in Hz
    Rate,
    /// Modulation depth (0.0 to 1.0)
    Depth,
    /// Delay time in ms
    Delay,
    /// Feedback amount
    Feedback,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
    /// Number of voices
    Voices,
}

/// Phaser effect parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PhaserParam {
    /// LFO rate in Hz
    Rate,
    /// Modulation depth (0.0 to 1.0)
    Depth,
    /// Feedback amount (-1.0 to 1.0)
    Feedback,
    /// Number of allpass stages (2-12)
    Stages,
    /// Center frequency in Hz
    CenterFreq,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
}

/// Flanger effect parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FlangerParam {
    /// LFO rate in Hz
    Rate,
    /// Modulation depth (0.0 to 1.0)
    Depth,
    /// Feedback amount (-1.0 to 1.0)
    Feedback,
    /// Base delay time in ms
    Delay,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
}

/// Compressor effect parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompressorParam {
    /// Threshold in dB (-60 to 0)
    Threshold,
    /// Compression ratio (1:1 to 20:1)
    Ratio,
    /// Attack time in ms
    Attack,
    /// Release time in ms
    Release,
    /// Makeup gain in dB
    Makeup,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
}

/// EQ effect parameters (3-band parametric).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EqParam {
    /// Low band frequency in Hz
    LowFreq,
    /// Low band gain in dB
    LowGain,
    /// Mid band frequency in Hz
    MidFreq,
    /// Mid band gain in dB
    MidGain,
    /// Mid band Q factor
    MidQ,
    /// High band frequency in Hz
    HighFreq,
    /// High band gain in dB
    HighGain,
    /// Dry/wet mix (0.0 to 1.0)
    Mix,
}

/// Sample player parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SamplePlayerParam {
    /// Playback speed (1.0 = normal)
    Speed,
    /// Start position (0.0 to 1.0)
    Start,
    /// End position (0.0 to 1.0)
    End,
    /// Loop mode
    LoopMode,
    /// Loop start position
    LoopStart,
    /// Loop end position
    LoopEnd,
    /// Output level
    Level,
}

/// Granular synth parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GranularParam {
    /// Position in sample (0.0 to 1.0)
    Position,
    /// Position randomization
    PositionSpread,
    /// Grain size in ms
    GrainSize,
    /// Grain density (grains per second)
    Density,
    /// Pitch shift in semitones
    Pitch,
    /// Pitch randomization
    PitchSpread,
    /// Grain shape/envelope
    Shape,
    /// Stereo spread
    Spread,
    /// Output level
    Level,
}

/// Oscilloscope visualizer parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OscilloscopeParam {
    /// Time scale (seconds per division)
    Time,
    /// Input gain
    Gain,
    /// Trigger level
    Trigger,
    /// Freeze display
    Frozen,
}

/// Level meter visualizer parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LevelMeterParam {
    /// Peak hold time (seconds)
    PeakHold,
    /// Meter decay rate
    DecayRate,
    /// Show RMS vs peak
    ShowRms,
}

// ============================================================================
// UNIFIED PARAMETER TYPE
// ============================================================================

/// A type-safe parameter identifier that encodes both module type and parameter.
///
/// This is the main type used throughout the system. The nested structure ensures
/// that you can't accidentally send an oscillator parameter to a filter module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TypedParam {
    Oscillator(OscillatorParam),
    MathOscillator(MathOscillatorParam),
    Filter(FilterParam),
    Envelope(EnvelopeParam),
    Lfo(LfoParam),
    Amplifier(AmplifierParam),
    Mixer(MixerParam),
    Delay(DelayParam),
    Reverb(ReverbParam),
    Distortion(DistortionParam),
    Chorus(ChorusParam),
    Phaser(PhaserParam),
    Flanger(FlangerParam),
    Compressor(CompressorParam),
    Eq(EqParam),
    SamplePlayer(SamplePlayerParam),
    Granular(GranularParam),
    Oscilloscope(OscilloscopeParam),
    LevelMeter(LevelMeterParam),
}

impl TypedParam {
    /// Get the module type this parameter belongs to.
    pub fn module_type(&self) -> ModuleType {
        match self {
            Self::Oscillator(_) => ModuleType::Oscillator,
            Self::MathOscillator(_) => ModuleType::MathOscillator,
            Self::Filter(_) => ModuleType::Filter,
            Self::Envelope(_) => ModuleType::Envelope,
            Self::Lfo(_) => ModuleType::Lfo,
            Self::Amplifier(_) => ModuleType::Amplifier,
            Self::Mixer(_) => ModuleType::Mixer,
            Self::Delay(_) => ModuleType::Delay,
            Self::Reverb(_) => ModuleType::Reverb,
            Self::Distortion(_) => ModuleType::Distortion,
            Self::Chorus(_) => ModuleType::Chorus,
            Self::Phaser(_) => ModuleType::Phaser,
            Self::Flanger(_) => ModuleType::Flanger,
            Self::Compressor(_) => ModuleType::Compressor,
            Self::Eq(_) => ModuleType::Eq,
            Self::SamplePlayer(_) => ModuleType::SamplePlayer,
            Self::Granular(_) => ModuleType::GranularSynth,
            Self::Oscilloscope(_) => ModuleType::Oscilloscope,
            Self::LevelMeter(_) => ModuleType::LevelMeter,
        }
    }
    
    /// Get the parameter name for display.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Oscillator(p) => match p {
                OscillatorParam::Waveform => "Waveform",
                OscillatorParam::Frequency => "Frequency",
                OscillatorParam::Detune => "Detune",
                OscillatorParam::Octave => "Octave",
                OscillatorParam::PulseWidth => "Pulse Width",
                OscillatorParam::Level => "Level",
                OscillatorParam::Phase => "Phase",
                OscillatorParam::FmMode => "FM Mode",
            },
            Self::MathOscillator(p) => match p {
                MathOscillatorParam::Algorithm => "Algorithm",
                MathOscillatorParam::Frequency => "Frequency",
                MathOscillatorParam::ParamA => "Param A",
                MathOscillatorParam::ParamB => "Param B",
                MathOscillatorParam::ParamC => "Param C",
                MathOscillatorParam::Level => "Level",
            },
            Self::Filter(p) => match p {
                FilterParam::Mode => "Mode",
                FilterParam::Cutoff => "Cutoff",
                FilterParam::Resonance => "Resonance",
                FilterParam::KeyTracking => "Key Tracking",
                FilterParam::Drive => "Drive",
                FilterParam::EnvAmount => "Env Amount",
            },
            Self::Envelope(p) => match p {
                EnvelopeParam::Attack => "Attack",
                EnvelopeParam::Decay => "Decay",
                EnvelopeParam::Sustain => "Sustain",
                EnvelopeParam::Release => "Release",
                EnvelopeParam::AttackCurve => "Attack Curve",
                EnvelopeParam::DecayCurve => "Decay Curve",
                EnvelopeParam::ReleaseCurve => "Release Curve",
                EnvelopeParam::VelocitySensitivity => "Velocity Sens",
            },
            Self::Lfo(p) => match p {
                LfoParam::Waveform => "Waveform",
                LfoParam::Rate => "Rate",
                LfoParam::Depth => "Depth",
                LfoParam::Phase => "Phase",
                LfoParam::TempoSync => "Tempo Sync",
                LfoParam::Retrigger => "Retrigger",
            },
            Self::Amplifier(p) => match p {
                AmplifierParam::Level => "Level",
                AmplifierParam::Pan => "Pan",
            },
            Self::Mixer(p) => match p {
                MixerParam::Input1 => "Input 1",
                MixerParam::Input2 => "Input 2",
                MixerParam::Input3 => "Input 3",
                MixerParam::Input4 => "Input 4",
                MixerParam::Master => "Master",
                MixerParam::Mute => "Mute",
                MixerParam::Limit => "Limiter",
            },
            Self::Delay(p) => match p {
                DelayParam::Mode => "Mode",
                DelayParam::Time => "Time",
                DelayParam::TimeLeft => "Time L",
                DelayParam::TimeRight => "Time R",
                DelayParam::Feedback => "Feedback",
                DelayParam::Mix => "Mix",
                DelayParam::Damping => "Damping",
                DelayParam::TempoSync => "Tempo Sync",
            },
            Self::Reverb(p) => match p {
                ReverbParam::RoomSize => "Room Size",
                ReverbParam::PreDelay => "Pre-Delay",
                ReverbParam::Damping => "Damping",
                ReverbParam::Width => "Width",
                ReverbParam::Mix => "Mix",
            },
            Self::Distortion(p) => match p {
                DistortionParam::Mode => "Mode",
                DistortionParam::Drive => "Drive",
                DistortionParam::Tone => "Tone",
                DistortionParam::Mix => "Mix",
            },
            Self::Chorus(p) => match p {
                ChorusParam::Rate => "Rate",
                ChorusParam::Depth => "Depth",
                ChorusParam::Delay => "Delay",
                ChorusParam::Feedback => "Feedback",
                ChorusParam::Mix => "Mix",
                ChorusParam::Voices => "Voices",
            },
            Self::Phaser(p) => match p {
                PhaserParam::Rate => "Rate",
                PhaserParam::Depth => "Depth",
                PhaserParam::Feedback => "Feedback",
                PhaserParam::Stages => "Stages",
                PhaserParam::CenterFreq => "Center Freq",
                PhaserParam::Mix => "Mix",
            },
            Self::Flanger(p) => match p {
                FlangerParam::Rate => "Rate",
                FlangerParam::Depth => "Depth",
                FlangerParam::Feedback => "Feedback",
                FlangerParam::Delay => "Delay",
                FlangerParam::Mix => "Mix",
            },
            Self::Compressor(p) => match p {
                CompressorParam::Threshold => "Threshold",
                CompressorParam::Ratio => "Ratio",
                CompressorParam::Attack => "Attack",
                CompressorParam::Release => "Release",
                CompressorParam::Makeup => "Makeup",
                CompressorParam::Mix => "Mix",
            },
            Self::Eq(p) => match p {
                EqParam::LowFreq => "Low Freq",
                EqParam::LowGain => "Low Gain",
                EqParam::MidFreq => "Mid Freq",
                EqParam::MidGain => "Mid Gain",
                EqParam::MidQ => "Mid Q",
                EqParam::HighFreq => "High Freq",
                EqParam::HighGain => "High Gain",
                EqParam::Mix => "Mix",
            },
            Self::SamplePlayer(p) => match p {
                SamplePlayerParam::Speed => "Speed",
                SamplePlayerParam::Start => "Start",
                SamplePlayerParam::End => "End",
                SamplePlayerParam::LoopMode => "Loop Mode",
                SamplePlayerParam::LoopStart => "Loop Start",
                SamplePlayerParam::LoopEnd => "Loop End",
                SamplePlayerParam::Level => "Level",
            },
            Self::Granular(p) => match p {
                GranularParam::Position => "Position",
                GranularParam::PositionSpread => "Pos Spread",
                GranularParam::GrainSize => "Grain Size",
                GranularParam::Density => "Density",
                GranularParam::Pitch => "Pitch",
                GranularParam::PitchSpread => "Pitch Spread",
                GranularParam::Shape => "Shape",
                GranularParam::Spread => "Spread",
                GranularParam::Level => "Level",
            },
            Self::Oscilloscope(p) => match p {
                OscilloscopeParam::Time => "Time",
                OscilloscopeParam::Gain => "Gain",
                OscilloscopeParam::Trigger => "Trigger",
                OscilloscopeParam::Frozen => "Frozen",
            },
            Self::LevelMeter(p) => match p {
                LevelMeterParam::PeakHold => "Peak Hold",
                LevelMeterParam::DecayRate => "Decay Rate",
                LevelMeterParam::ShowRms => "Show RMS",
            },
        }
    }
}

// ============================================================================
// TYPE-SAFE PARAMETER VALUE
// ============================================================================

/// A type-safe parameter value that uses proper enums instead of strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TypedValue {
    /// Floating point value
    Float(f32),
    /// Integer value
    Int(i32),
    /// Boolean value
    Bool(bool),
    /// Oscillator waveform
    Waveform(Waveform),
    /// LFO waveform
    LfoWaveform(LfoWaveform),
    /// Filter mode
    FilterMode(FilterMode),
    /// Delay mode
    DelayMode(DelayMode),
    /// Distortion mode
    DistortionMode(DistortionMode),
    /// Loop mode
    LoopMode(LoopMode),
    /// Math oscillator algorithm
    MathAlgo(MathAlgo),
}

impl TypedValue {
    /// Get as float, converting if possible.
    pub fn as_float(&self) -> Option<f32> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(i) => Some(*i as f32),
            Self::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }
    
    /// Get as int, converting if possible.
    pub fn as_int(&self) -> Option<i32> {
        match self {
            Self::Float(f) => Some(*f as i32),
            Self::Int(i) => Some(*i),
            Self::Bool(b) => Some(if *b { 1 } else { 0 }),
            Self::Waveform(w) => Some(*w as i32),
            Self::LfoWaveform(w) => Some(*w as i32),
            Self::FilterMode(m) => Some(*m as i32),
            Self::DelayMode(m) => Some(*m as i32),
            Self::DistortionMode(m) => Some(*m as i32),
            Self::LoopMode(m) => Some(*m as i32),
            Self::MathAlgo(a) => Some(*a as i32),
        }
    }
}

impl From<f32> for TypedValue {
    fn from(v: f32) -> Self {
        Self::Float(v)
    }
}

impl From<i32> for TypedValue {
    fn from(v: i32) -> Self {
        Self::Int(v)
    }
}

impl From<bool> for TypedValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<Waveform> for TypedValue {
    fn from(v: Waveform) -> Self {
        Self::Waveform(v)
    }
}

impl From<LfoWaveform> for TypedValue {
    fn from(v: LfoWaveform) -> Self {
        Self::LfoWaveform(v)
    }
}

impl From<FilterMode> for TypedValue {
    fn from(v: FilterMode) -> Self {
        Self::FilterMode(v)
    }
}

impl From<MathAlgo> for TypedValue {
    fn from(v: MathAlgo) -> Self {
        Self::MathAlgo(v)
    }
}

// ============================================================================
// TYPE-SAFE PORT IDENTIFIERS
// ============================================================================

/// Standard audio port types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AudioPort {
    /// Mono input
    In,
    /// Mono output
    Out,
    /// Left channel input
    InLeft,
    /// Right channel input  
    InRight,
    /// Left channel output
    OutLeft,
    /// Right channel output
    OutRight,
}

/// Standard control/CV port types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ControlPort {
    /// Frequency CV input (1V/octave)
    FrequencyCV,
    /// Amplitude CV input
    AmplitudeCV,
    /// Cutoff frequency CV
    CutoffCV,
    /// Resonance CV
    ResonanceCV,
    /// Gate input
    Gate,
    /// Trigger input
    Trigger,
    /// Generic modulation input
    ModIn,
    /// Envelope output
    EnvOut,
    /// LFO output
    LfoOut,
    /// PWM input for pulse width modulation
    PwmIn,
    /// Sync input for hard sync
    SyncIn,
}

/// A type-safe port identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Port {
    Audio(AudioPort),
    Control(ControlPort),
}

impl Port {
    /// Get the display name for this port.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Audio(p) => match p {
                AudioPort::In => "In",
                AudioPort::Out => "Out",
                AudioPort::InLeft => "In L",
                AudioPort::InRight => "In R",
                AudioPort::OutLeft => "Out L",
                AudioPort::OutRight => "Out R",
            },
            Self::Control(p) => match p {
                ControlPort::FrequencyCV => "Freq CV",
                ControlPort::AmplitudeCV => "Amp CV",
                ControlPort::CutoffCV => "Cutoff CV",
                ControlPort::ResonanceCV => "Res CV",
                ControlPort::Gate => "Gate",
                ControlPort::Trigger => "Trig",
                ControlPort::ModIn => "Mod In",
                ControlPort::EnvOut => "Env Out",
                ControlPort::LfoOut => "LFO Out",
                ControlPort::PwmIn => "PWM",
                ControlPort::SyncIn => "Sync",
            },
        }
    }
    
    /// Check if this is an input port.
    pub fn is_input(&self) -> bool {
        match self {
            Self::Audio(p) => matches!(p, AudioPort::In | AudioPort::InLeft | AudioPort::InRight),
            Self::Control(p) => !matches!(p, ControlPort::EnvOut | ControlPort::LfoOut),
        }
    }
    
    /// Check if this is an output port.
    pub fn is_output(&self) -> bool {
        !self.is_input()
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_typed_param_module_type() {
        let osc_param = TypedParam::Oscillator(OscillatorParam::Waveform);
        assert_eq!(osc_param.module_type(), ModuleType::Oscillator);
        
        let filter_param = TypedParam::Filter(FilterParam::Cutoff);
        assert_eq!(filter_param.module_type(), ModuleType::Filter);
    }
    
    #[test]
    fn test_typed_value_conversion() {
        let waveform = TypedValue::Waveform(Waveform::Sawtooth);
        assert_eq!(waveform.as_int(), Some(2)); // Sawtooth is index 2
        
        let float_val = TypedValue::Float(0.5);
        assert_eq!(float_val.as_float(), Some(0.5));
    }
    
    #[test]
    fn test_port_direction() {
        let input = Port::Audio(AudioPort::In);
        assert!(input.is_input());
        assert!(!input.is_output());
        
        let output = Port::Audio(AudioPort::Out);
        assert!(!output.is_input());
        assert!(output.is_output());
    }
}
