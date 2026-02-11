//! Type-safe parameter system for the modular synthesizer.
//!
//! This module provides strongly-typed parameters where each parameter
//! variant carries its properly typed value (e.g., `Frequency(Hertz)`).
//!
//! # Design Philosophy
//!
//! Each parameter is a data-carrying enum variant. This means:
//! - Type safety: Can't send wrong parameter to wrong module
//! - Value safety: Values are wrapped in domain types (Hertz, Gain, etc.)
//! - GUI compatibility: `as_f32()` and `with_f32()` for sliders
//! - Comparison: `same_kind()` to compare param types ignoring values

mod effects;
mod envelopes;
mod filters;
mod lfo;
mod modules;
mod noise;
mod oscillators;
mod physical;
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
pub use modules::{AmplifierParam, LevelMeterParam, MixerParam, OscilloscopeParam};
pub use noise::{NoiseParam, NoiseType};
pub use oscillators::{FmMode, MathAlgo, MathOscillatorParam, OscillatorParam, Waveform};
pub use physical::{
    BodyResonanceParam, KeyboardPannerParam, MechanicalNoiseParam, MechanicalNoiseType,
};
pub use sub_osc::{SubOscOctave, SubOscParam, SubOscWaveform};

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
    // Visualizers
    Oscilloscope,
    LevelMeter,
    // Physical modeling
    KeyboardPanner,
    BodyResonance,
    MechanicalNoise,
}

impl ModuleType {
    // ========================================================================
    // MODULE CLASSIFICATION METHODS
    // ========================================================================

    /// Check if this module type is a voice module (polyphonic).
    ///
    /// Voice modules are duplicated per voice and process within the voice allocator.
    /// They exist once per active voice.
    #[inline]
    pub fn is_voice_module(&self) -> bool {
        matches!(
            self,
            Self::Oscillator
                | Self::MathOscillator
                | Self::SubOscillator
                | Self::Noise
                | Self::Filter
                | Self::Envelope
                | Self::Lfo
                | Self::Amplifier
                | Self::Mixer
                | Self::StereoOutput
                // Physical modeling modules (per-voice)
                | Self::KeyboardPanner
                | Self::BodyResonance
                | Self::MechanicalNoise
        )
    }

    /// Check if this module type is an audio effect.
    ///
    /// Effects process audio in series after voice mixing. They exist once
    /// per instrument in the effect chain.
    #[inline]
    pub fn is_effect(&self) -> bool {
        matches!(
            self,
            Self::Delay
                | Self::Reverb
                | Self::Distortion
                | Self::Chorus
                | Self::Phaser
                | Self::Flanger
                | Self::Compressor
                | Self::Eq
        )
    }

    /// Check if this module type is a visualizer.
    ///
    /// Visualizers capture audio data for display but don't modify the signal.
    #[inline]
    pub fn is_visualizer(&self) -> bool {
        matches!(self, Self::Oscilloscope | Self::LevelMeter)
    }

    /// Check if this module type is global (not a voice module).
    ///
    /// Global modules (effects, visualizers) exist once per instrument
    /// and process audio after voice mixing.
    #[inline]
    pub fn is_global(&self) -> bool {
        !self.is_voice_module()
    }

    // ========================================================================
    // DISPLAY AND CONVERSION METHODS
    // ========================================================================

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
            Self::Oscilloscope => "Oscilloscope",
            Self::LevelMeter => "Level Meter",
            // Physical modeling
            Self::KeyboardPanner => "Keyboard Panner",
            Self::BodyResonance => "Body Resonance",
            Self::MechanicalNoise => "Mechanical Noise",
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
            Self::Oscilloscope => "scp",
            Self::LevelMeter => "mtr",
            // Physical modeling
            Self::KeyboardPanner => "kbp",
            Self::BodyResonance => "bdy",
            Self::MechanicalNoise => "mec",
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
            "scp" => Some(Self::Oscilloscope),
            "mtr" => Some(Self::LevelMeter),
            // Physical modeling
            "kbp" => Some(Self::KeyboardPanner),
            "bdy" => Some(Self::BodyResonance),
            "mec" => Some(Self::MechanicalNoise),
            _ => None,
        }
    }

    // Note: to_patch_module_type() moved to modular_synth crate
    // since PatchModuleType is crate-specific for serialization
}

// ============================================================================
// UNIFIED PARAMETER TYPE (with embedded values)
// ============================================================================

/// A type-safe parameter with its value embedded.
///
/// Each variant carries a module-specific parameter which itself contains
/// the properly typed value. For example:
/// - `Param::Oscillator(OscillatorParam::Frequency(Hertz::new(440.0)))`
/// - `Param::Filter(FilterParam::Cutoff(Hertz::new(1000.0)))`
///
/// This design eliminates the need for separate ID and value types.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Param {
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
    Oscilloscope(OscilloscopeParam),
    LevelMeter(LevelMeterParam),
    // Physical modeling
    KeyboardPanner(KeyboardPannerParam),
    BodyResonance(BodyResonanceParam),
    MechanicalNoise(MechanicalNoiseParam),
}

impl Param {
    /// Check if two parameters are the same kind (ignoring values).
    ///
    /// This is useful for finding/updating parameters in a list.
    /// ```ignore
    /// let params: Vec<Param> = get_params();
    /// let target = Param::Oscillator(OscillatorParam::frequency_default());
    /// if let Some(p) = params.iter().find(|p| p.same_kind(&target)) {
    ///     // Found the frequency parameter
    /// }
    /// ```
    pub fn same_kind(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Oscillator(a), Self::Oscillator(b)) => a.same_kind(b),
            (Self::MathOscillator(a), Self::MathOscillator(b)) => a.same_kind(b),
            (Self::SubOsc(a), Self::SubOsc(b)) => a.same_kind(b),
            (Self::Noise(a), Self::Noise(b)) => a.same_kind(b),
            (Self::Filter(a), Self::Filter(b)) => a.same_kind(b),
            (Self::Envelope(a), Self::Envelope(b)) => a.same_kind(b),
            (Self::Lfo(a), Self::Lfo(b)) => a.same_kind(b),
            (Self::Amplifier(a), Self::Amplifier(b)) => a.same_kind(b),
            (Self::Mixer(a), Self::Mixer(b)) => a.same_kind(b),
            (Self::Delay(a), Self::Delay(b)) => a.same_kind(b),
            (Self::Reverb(a), Self::Reverb(b)) => a.same_kind(b),
            (Self::Distortion(a), Self::Distortion(b)) => a.same_kind(b),
            (Self::Chorus(a), Self::Chorus(b)) => a.same_kind(b),
            (Self::Phaser(a), Self::Phaser(b)) => a.same_kind(b),
            (Self::Flanger(a), Self::Flanger(b)) => a.same_kind(b),
            (Self::Compressor(a), Self::Compressor(b)) => a.same_kind(b),
            (Self::Eq(a), Self::Eq(b)) => a.same_kind(b),
            (Self::Oscilloscope(a), Self::Oscilloscope(b)) => a.same_kind(b),
            (Self::LevelMeter(a), Self::LevelMeter(b)) => a.same_kind(b),
            // Physical modeling
            (Self::KeyboardPanner(a), Self::KeyboardPanner(b)) => a.same_kind(b),
            (Self::BodyResonance(a), Self::BodyResonance(b)) => a.same_kind(b),
            (Self::MechanicalNoise(a), Self::MechanicalNoise(b)) => a.same_kind(b),
            _ => false,
        }
    }

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
            Self::Oscilloscope(_) => ModuleType::Oscilloscope,
            Self::LevelMeter(_) => ModuleType::LevelMeter,
            // Physical modeling
            Self::KeyboardPanner(_) => ModuleType::KeyboardPanner,
            Self::BodyResonance(_) => ModuleType::BodyResonance,
            Self::MechanicalNoise(_) => ModuleType::MechanicalNoise,
        }
    }

    /// Get the parameter name for display.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Oscillator(p) => p.name(),
            Self::MathOscillator(p) => p.name(),
            Self::SubOsc(p) => p.name(),
            Self::Noise(p) => p.name(),
            Self::Filter(p) => p.name(),
            Self::Envelope(p) => p.name(),
            Self::Lfo(p) => p.name(),
            Self::Amplifier(p) => p.name(),
            Self::Mixer(p) => p.name(),
            Self::Delay(p) => p.name(),
            Self::Reverb(p) => p.name(),
            Self::Distortion(p) => p.name(),
            Self::Chorus(p) => p.name(),
            Self::Phaser(p) => p.name(),
            Self::Flanger(p) => p.name(),
            Self::Compressor(p) => p.name(),
            Self::Eq(p) => p.name(),
            Self::Oscilloscope(p) => p.name(),
            Self::LevelMeter(p) => p.name(),
            // Physical modeling
            Self::KeyboardPanner(p) => p.name(),
            Self::BodyResonance(p) => p.name(),
            Self::MechanicalNoise(p) => p.name(),
        }
    }

    /// Get the value as f32 (for GUI sliders).
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Oscillator(p) => p.as_f32(),
            Self::MathOscillator(p) => p.as_f32(),
            Self::SubOsc(p) => p.as_f32(),
            Self::Noise(p) => p.as_f32(),
            Self::Filter(p) => p.as_f32(),
            Self::Envelope(p) => p.as_f32(),
            Self::Lfo(p) => p.as_f32(),
            Self::Amplifier(p) => p.as_f32(),
            Self::Mixer(p) => p.as_f32(),
            Self::Delay(p) => p.as_f32(),
            Self::Reverb(p) => p.as_f32(),
            Self::Distortion(p) => p.as_f32(),
            Self::Chorus(p) => p.as_f32(),
            Self::Phaser(p) => p.as_f32(),
            Self::Flanger(p) => p.as_f32(),
            Self::Compressor(p) => p.as_f32(),
            Self::Eq(p) => p.as_f32(),
            Self::Oscilloscope(p) => p.as_f32(),
            Self::LevelMeter(p) => p.as_f32(),
            // Physical modeling
            Self::KeyboardPanner(p) => p.as_f32(),
            Self::BodyResonance(p) => p.as_f32(),
            Self::MechanicalNoise(p) => p.as_f32(),
        }
    }

    /// Create the same parameter kind with a new f32 value (for GUI sliders).
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Oscillator(p) => Self::Oscillator(p.with_f32(value)),
            Self::MathOscillator(p) => Self::MathOscillator(p.with_f32(value)),
            Self::SubOsc(p) => Self::SubOsc(p.with_f32(value)),
            Self::Noise(p) => Self::Noise(p.with_f32(value)),
            Self::Filter(p) => Self::Filter(p.with_f32(value)),
            Self::Envelope(p) => Self::Envelope(p.with_f32(value)),
            Self::Lfo(p) => Self::Lfo(p.with_f32(value)),
            Self::Amplifier(p) => Self::Amplifier(p.with_f32(value)),
            Self::Mixer(p) => Self::Mixer(p.with_f32(value)),
            Self::Delay(p) => Self::Delay(p.with_f32(value)),
            Self::Reverb(p) => Self::Reverb(p.with_f32(value)),
            Self::Distortion(p) => Self::Distortion(p.with_f32(value)),
            Self::Chorus(p) => Self::Chorus(p.with_f32(value)),
            Self::Phaser(p) => Self::Phaser(p.with_f32(value)),
            Self::Flanger(p) => Self::Flanger(p.with_f32(value)),
            Self::Compressor(p) => Self::Compressor(p.with_f32(value)),
            Self::Eq(p) => Self::Eq(p.with_f32(value)),
            Self::Oscilloscope(p) => Self::Oscilloscope(p.with_f32(value)),
            Self::LevelMeter(p) => Self::LevelMeter(p.with_f32(value)),
            // Physical modeling
            Self::KeyboardPanner(p) => Self::KeyboardPanner(p.with_f32(value)),
            Self::BodyResonance(p) => Self::BodyResonance(p.with_f32(value)),
            Self::MechanicalNoise(p) => Self::MechanicalNoise(p.with_f32(value)),
        }
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
    use crate::types::Hertz;

    #[test]
    fn test_module_type_is_voice_module() {
        // Voice modules (should be true)
        assert!(ModuleType::Oscillator.is_voice_module());
        assert!(ModuleType::MathOscillator.is_voice_module());
        assert!(ModuleType::SubOscillator.is_voice_module());
        assert!(ModuleType::Noise.is_voice_module());
        assert!(ModuleType::Filter.is_voice_module());
        assert!(ModuleType::Envelope.is_voice_module());
        assert!(ModuleType::Lfo.is_voice_module());
        assert!(ModuleType::Amplifier.is_voice_module());
        assert!(ModuleType::Mixer.is_voice_module());
        assert!(ModuleType::StereoOutput.is_voice_module());

        // Global modules (should be false)
        assert!(!ModuleType::Delay.is_voice_module());
        assert!(!ModuleType::Reverb.is_voice_module());
        assert!(!ModuleType::Oscilloscope.is_voice_module());
    }

    #[test]
    fn test_module_type_is_effect() {
        // Effects (should be true)
        assert!(ModuleType::Delay.is_effect());
        assert!(ModuleType::Reverb.is_effect());
        assert!(ModuleType::Distortion.is_effect());
        assert!(ModuleType::Chorus.is_effect());
        assert!(ModuleType::Phaser.is_effect());
        assert!(ModuleType::Flanger.is_effect());
        assert!(ModuleType::Compressor.is_effect());
        assert!(ModuleType::Eq.is_effect());

        // Non-effects (should be false)
        assert!(!ModuleType::Oscillator.is_effect());
        assert!(!ModuleType::Oscilloscope.is_effect());
    }

    #[test]
    fn test_module_type_is_visualizer() {
        assert!(ModuleType::Oscilloscope.is_visualizer());
        assert!(ModuleType::LevelMeter.is_visualizer());
        assert!(!ModuleType::Oscillator.is_visualizer());
        assert!(!ModuleType::Delay.is_visualizer());
    }

    #[test]
    fn test_module_type_is_global() {
        // Global = !is_voice_module
        assert!(ModuleType::Delay.is_global());
        assert!(ModuleType::Oscilloscope.is_global());
        assert!(!ModuleType::Oscillator.is_global());
        assert!(!ModuleType::Filter.is_global());
    }

    #[test]
    fn test_param_module_type() {
        let osc_param = Param::Oscillator(OscillatorParam::Waveform(Waveform::default()));
        assert_eq!(osc_param.module_type(), ModuleType::Oscillator);

        let filter_param = Param::Filter(FilterParam::Cutoff(Hertz::new(1000.0)));
        assert_eq!(filter_param.module_type(), ModuleType::Filter);
    }

    #[test]
    fn test_param_same_kind() {
        let freq1 = Param::Oscillator(OscillatorParam::Frequency(Hertz::new(440.0)));
        let freq2 = Param::Oscillator(OscillatorParam::Frequency(Hertz::new(880.0)));
        let waveform = Param::Oscillator(OscillatorParam::Waveform(Waveform::Sine));

        assert!(freq1.same_kind(&freq2));
        assert!(!freq1.same_kind(&waveform));
    }

    #[test]
    fn test_param_as_f32_with_f32() {
        let freq = Param::Oscillator(OscillatorParam::Frequency(Hertz::new(440.0)));
        assert_eq!(freq.as_f32(), 440.0);

        let new_freq = freq.with_f32(880.0);
        assert_eq!(new_freq.as_f32(), 880.0);
        assert!(freq.same_kind(&new_freq));
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
