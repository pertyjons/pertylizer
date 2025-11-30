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

mod effects;
mod envelopes;
mod filters;
mod lfo;
mod modules;
mod noise;
mod oscillators;
mod sub_osc;

use serde::{Deserialize, Serialize};

// Re-export all parameter types
pub use effects::{
    ChorusParam, CompressorParam, DelayMode, DelayParam, DistortionMode, DistortionParam, EqParam,
    FlangerParam, PhaserParam, ReverbParam,
};
pub use envelopes::EnvelopeParam;
pub use filters::{FilterMode, FilterParam};
pub use lfo::{LfoParam, LfoWaveform};
pub use modules::{
    AmplifierParam, GranularParam, LevelMeterParam, LoopMode, MixerParam, OscilloscopeParam,
    SamplePlayerParam,
};
pub use noise::NoiseParam;
pub use oscillators::{MathAlgo, MathOscillatorParam, OscillatorParam, Waveform};
pub use sub_osc::SubOscParam;

// ============================================================================
// MODULE TYPE ENUM
// ============================================================================

/// Strongly-typed module type identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModuleType {
    Oscillator,
    MathOscillator,
    SubOscillator,
    Noise,
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
            Self::SubOscillator => "Sub Oscillator",
            Self::Noise => "Noise",
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
            Self::SubOscillator => "sub",
            Self::Noise => "nse",
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
            "sub" => Some(Self::SubOscillator),
            "nse" => Some(Self::Noise),
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
            Self::SubOscillator => PM::SubOscillator,
            Self::Noise => PM::Noise,
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
            Self::Phaser
            | Self::Flanger
            | Self::Compressor
            | Self::Eq
            | Self::SamplePlayer
            | Self::SampleRecorder
            | Self::GranularSynth
            | Self::Wavetable => PM::Mixer,
        }
    }
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
    SubOsc(SubOscParam),
    Noise(NoiseParam),
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
            Self::SubOsc(_) => ModuleType::SubOscillator,
            Self::Noise(_) => ModuleType::Noise,
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
            Self::SubOsc(p) => match p {
                SubOscParam::Waveform => "Waveform",
                SubOscParam::Octave => "Octave",
                SubOscParam::Level => "Level",
            },
            Self::Noise(p) => match p {
                NoiseParam::Type => "Type",
                NoiseParam::Level => "Level",
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
                LfoParam::SyncDivision => "Division",
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
                DelayParam::SyncDivision => "Division",
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
