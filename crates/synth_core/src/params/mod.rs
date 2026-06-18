//! Type-safe parameter system for Pertylizer.
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

mod additive;
mod am_formant;
mod beat_detector;
mod chaotic_osc;
mod convolver;
mod drift_generator;
mod effects;
mod envelope_follower;
mod envelopes;
mod filters;
mod fof;
mod fooglers;
mod formant_filter;
mod fractal_osc;
mod frequency_shifter;
mod generative;
mod granular;
mod kinetic;
mod la_synth;
mod lfo;
mod mod_matrix;
mod modules;
mod mseg;
mod noise;
mod oscillators;
mod padsynth;
mod phase_vocoder;
mod physical;
mod pitch_tracker;
mod ring_mod;
mod sampler;
mod signal_monitor;
mod spectrum_analyzer;
mod sub_osc;
mod vector_mixer;
mod vocal_tract;
mod voice_synth;
mod waveshaper;
mod wavetable;

use serde::{Deserialize, Serialize};

// Re-export all parameter types
pub use additive::AdditiveParam;
pub use am_formant::AmFormantParam;
pub use beat_detector::BeatDetectorParam;
pub use chaotic_osc::{ChaoticOscParam, ChaoticSystem};
pub use convolver::{ConvolverParam, ImpulseResponse};
pub use drift_generator::DriftGeneratorParam;
pub use effects::{
    BbdDelayParam, ChorusParam, CompressorParam, CrossoverParam, DelayMode, DelayParam,
    DistortionMode, DistortionParam, EnsembleChorusParam, EqParam, FlangerParam, GranularFxParam,
    LimiterParam, MidSideParam, ModalResonatorParam, PhaserParam, ReverbParam, ReverseGateMode,
    ReverseGateReverbParam, ReverseGateTrigger, ShimmerReverbParam, SpectralBlurParam, TiltEqParam,
    UnivibeParam, VocoderParam,
};
pub use envelope_follower::EnvelopeFollowerParam;
pub use envelopes::EnvelopeParam;
pub use filters::{FilterMode, FilterModel, FilterParam};
pub use fof::FofParam;
pub use fooglers::FooglersParam;
pub use formant_filter::FormantFilterParam;
pub use fractal_osc::FractalOscParam;
pub use frequency_shifter::FrequencyShifterParam;
pub use generative::{EuclideanParam, RandomGatesParam, TuringMachineParam, TuringScale};
pub use granular::{GrainSource, GrainWindow, GranularParam};
pub use kinetic::{
    EasingCurve, KineticLoopMode, KineticParam, easing_acceleration, easing_position,
    easing_velocity,
};
pub use la_synth::LaSynthParam;
pub use lfo::{LfoParam, LfoWaveform};
pub use mod_matrix::{
    DestAddr, MAX_MOD_MATRIX_SLOTS, MacroSource, ModDestination, ModMatrixGridSize, ModMatrixParam,
    ModRouting, ModSource, SrcAddr,
};
pub use modules::{AmplifierParam, LevelMeterParam, MixerParam, OscilloscopeParam};
pub use mseg::MsegParam;
pub use noise::{NoiseParam, NoiseType};
pub use oscillators::{
    AntiAliasMode, FmMode, MathAlgo, MathOscillatorParam, OscillatorParam, Waveform,
};
pub use padsynth::PadSynthParam;
pub use phase_vocoder::{FftSizeOption, PhaseVocoderParam};
pub use physical::{
    BodyResonanceParam, KeyboardPannerParam, MechanicalNoiseParam, MechanicalNoiseType,
};
pub use pitch_tracker::PitchTrackerParam;
pub use ring_mod::RingModParam;
pub use sampler::{PlayDirection, SampleId, SamplerParam, SamplerPlayMode};
pub use signal_monitor::SignalMonitorParam;
pub use spectrum_analyzer::SpectrumAnalyzerParam;
pub use sub_osc::{SubOscOctave, SubOscParam, SubOscWaveform};
pub use vector_mixer::VectorMixerParam;
pub use vocal_tract::VocalTractParam;
pub use voice_synth::VoiceSynthParam;
pub use waveshaper::{WaveshaperCurve, WaveshaperParam};
pub use wavetable::{WavetableParam, WavetableSelect};

// ============================================================================
// MODULE TYPE ENUM
// ============================================================================

/// Strongly-typed module type identifier.
///
/// Variant declaration order is load-bearing: `Ord` is derived, and
/// `ModuleGraph` orders nodes by `(ModuleType, instance)` to keep offline
/// renders bit-exact across calls. Reordering or inserting variants
/// changes the processing order for any voice graph that mixes the
/// affected types, which silently changes audio output for existing
/// patches. Append new variants at the end.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[schemars(rename_all = "snake_case")]
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
    Waveshaper,
    // Visualizers
    Oscilloscope,
    LevelMeter,
    SpectrumAnalyzer,
    // Modulation
    ModMatrix,
    // Modulation / Utility
    RingMod,
    EnvelopeFollower,
    WavetableOsc,
    Mseg,
    AdditiveOsc,
    // Effects (continued)
    BbdDelay,
    MidSide,
    Limiter,
    // Generative
    Euclidean,
    TuringMachine,
    RandomGates,
    // Physical modeling
    KeyboardPanner,
    BodyResonance,
    MechanicalNoise,
    // Granular
    GranularOsc,
    // Spectral effects
    Convolver,
    PhaseVocoder,
    // Kinetic modulation
    KineticModulator,
    // Signal monitor (inline voice-level visualizer)
    SignalMonitor,
    // Spectral effects (continued)
    FrequencyShifter,
    // Synthesis
    VectorMixer,
    LaSynth,
    // Utility
    PitchTracker,
    // New effects (v0.189.0)
    EnsembleChorus,
    ShimmerReverb,
    GranularFx,
    SpectralBlur,
    ModalResonator,
    ReverseGateReverb,
    // Fractal synthesis
    FractalOsc,
    // Sampler
    Sampler,
    // Audio input
    AudioInput,
    // MusicDSP additions (voice modules)
    LadderFilter,
    DriftGenerator,
    ChaoticOsc,
    FormantFilter,
    Fooglers,
    BeatDetector,
    PadSynth,
    AmFormant,
    // MusicDSP additions (effects)
    TiltEq,
    Univibe,
    CrossoverSplitter,
    Vocoder,
    // Synthesis (voice)
    VoiceSynth,
    VocalTract,
    Fof,
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
                | Self::ModMatrix
                // Modulation / Utility
                | Self::RingMod
                | Self::EnvelopeFollower
                | Self::WavetableOsc
                | Self::Mseg
                | Self::AdditiveOsc
                // Generative (per-voice)
                | Self::Euclidean
                | Self::TuringMachine
                | Self::RandomGates
                // Physical modeling modules (per-voice)
                | Self::KeyboardPanner
                | Self::BodyResonance
                | Self::MechanicalNoise
                // Granular
                | Self::GranularOsc
                // Kinetic modulation
                | Self::KineticModulator
                // Signal monitor (inline visualizer in voice graph)
                | Self::SignalMonitor
                // Synthesis
                | Self::VectorMixer
                | Self::LaSynth
                // Utility
                | Self::PitchTracker
                // Fractal synthesis
                | Self::FractalOsc
                // Sampler
                | Self::Sampler
                // Audio input
                | Self::AudioInput
                // MusicDSP additions
                | Self::LadderFilter
                | Self::DriftGenerator
                | Self::ChaoticOsc
                | Self::FormantFilter
                | Self::Fooglers
                | Self::BeatDetector
                | Self::PadSynth
                | Self::AmFormant
                | Self::VoiceSynth
                | Self::VocalTract
                | Self::Fof
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
                | Self::Waveshaper
                | Self::BbdDelay
                | Self::MidSide
                | Self::Limiter
                | Self::Convolver
                | Self::PhaseVocoder
                | Self::FrequencyShifter
                | Self::EnsembleChorus
                | Self::ShimmerReverb
                | Self::GranularFx
                | Self::SpectralBlur
                | Self::ModalResonator
                | Self::ReverseGateReverb
                // MusicDSP additions (effects)
                | Self::TiltEq
                | Self::Univibe
                | Self::CrossoverSplitter
                | Self::Vocoder
        )
    }

    /// Check if this module type is a visualizer.
    ///
    /// Visualizers capture audio data for display but don't modify the signal.
    #[inline]
    pub fn is_visualizer(&self) -> bool {
        matches!(
            self,
            Self::Oscilloscope | Self::LevelMeter | Self::SpectrumAnalyzer
        )
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
            Self::Waveshaper => "Waveshaper",
            Self::Oscilloscope => "Oscilloscope",
            Self::LevelMeter => "Level Meter",
            Self::SpectrumAnalyzer => "Spectrum Analyzer",
            Self::ModMatrix => "Mod Matrix",
            // Modulation / Utility
            Self::RingMod => "Ring Mod",
            Self::EnvelopeFollower => "Env Follower",
            Self::WavetableOsc => "Wavetable",
            Self::Mseg => "MSEG",
            Self::AdditiveOsc => "Additive",
            // Effects (continued)
            Self::BbdDelay => "BBD Delay",
            Self::MidSide => "Mid/Side",
            Self::Limiter => "Limiter",
            // Generative
            Self::Euclidean => "Euclidean",
            Self::TuringMachine => "Turing Machine",
            Self::RandomGates => "Random Gates",
            // Physical modeling
            Self::KeyboardPanner => "Keyboard Panner",
            Self::BodyResonance => "Body Resonance",
            Self::MechanicalNoise => "Mechanical Noise",
            Self::GranularOsc => "Granular",
            Self::Convolver => "Convolver",
            Self::PhaseVocoder => "Phase Vocoder",
            Self::KineticModulator => "Kinetic Mod",
            Self::SignalMonitor => "Signal Monitor",
            Self::FrequencyShifter => "Freq Shifter",
            Self::VectorMixer => "Vector Mixer",
            Self::LaSynth => "LA Synth",
            Self::PitchTracker => "Pitch Tracker",
            Self::EnsembleChorus => "Ensemble Chorus",
            Self::ShimmerReverb => "Shimmer Reverb",
            Self::GranularFx => "Granular FX",
            Self::SpectralBlur => "Spectral Blur",
            Self::ModalResonator => "Modal Resonator",
            Self::ReverseGateReverb => "Reverse/Gate Reverb",
            Self::FractalOsc => "Fractal Osc",
            Self::Sampler => "Sampler",
            Self::AudioInput => "Audio Input",
            // MusicDSP additions
            Self::LadderFilter => "Ladder Filter",
            Self::DriftGenerator => "Drift Generator",
            Self::ChaoticOsc => "Chaotic Osc",
            Self::FormantFilter => "Formant Filter",
            Self::Fooglers => "Fooglers",
            Self::BeatDetector => "Beat Detector",
            Self::PadSynth => "PADsynth",
            Self::AmFormant => "AM Formant",
            Self::TiltEq => "Tilt EQ",
            Self::Univibe => "Univibe",
            Self::CrossoverSplitter => "Crossover Splitter",
            Self::Vocoder => "LPC Vocoder",
            Self::VoiceSynth => "Voice Synth",
            Self::VocalTract => "Vocal Tract",
            Self::Fof => "FOF",
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
            Self::Waveshaper => "wsh",
            Self::Oscilloscope => "scp",
            Self::LevelMeter => "mtr",
            Self::SpectrumAnalyzer => "spa",
            Self::ModMatrix => "mmx",
            // Modulation / Utility
            Self::RingMod => "rng",
            Self::EnvelopeFollower => "efl",
            Self::WavetableOsc => "wtb",
            Self::Mseg => "msg",
            Self::AdditiveOsc => "add",
            // Effects (continued)
            Self::BbdDelay => "bbd",
            Self::MidSide => "mds",
            Self::Limiter => "lmt",
            // Generative
            Self::Euclidean => "euc",
            Self::TuringMachine => "tur",
            Self::RandomGates => "rgt",
            // Physical modeling
            Self::KeyboardPanner => "kbp",
            Self::BodyResonance => "bdy",
            Self::MechanicalNoise => "mec",
            Self::GranularOsc => "grn",
            Self::Convolver => "cnv",
            Self::PhaseVocoder => "pvc",
            Self::KineticModulator => "kin",
            Self::SignalMonitor => "smn",
            Self::FrequencyShifter => "fsf",
            Self::VectorMixer => "vec",
            Self::LaSynth => "las",
            Self::PitchTracker => "ptr",
            Self::EnsembleChorus => "enc",
            Self::ShimmerReverb => "shr",
            Self::GranularFx => "gfx",
            Self::SpectralBlur => "sbl",
            Self::ModalResonator => "mdr",
            Self::ReverseGateReverb => "rgr",
            Self::FractalOsc => "frc",
            Self::Sampler => "sam",
            Self::AudioInput => "ain",
            // MusicDSP additions
            Self::LadderFilter => "ldr",
            Self::DriftGenerator => "drf",
            Self::ChaoticOsc => "cha",
            Self::FormantFilter => "fmt",
            Self::Fooglers => "fog",
            Self::BeatDetector => "btd",
            Self::PadSynth => "pad",
            Self::AmFormant => "amf",
            Self::TiltEq => "teq",
            Self::Univibe => "uvb",
            Self::CrossoverSplitter => "cxo",
            Self::Vocoder => "vcd",
            Self::VoiceSynth => "vox",
            Self::VocalTract => "vtr",
            Self::Fof => "fof",
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
            "wsh" => Some(Self::Waveshaper),
            "scp" => Some(Self::Oscilloscope),
            "mtr" => Some(Self::LevelMeter),
            "spa" => Some(Self::SpectrumAnalyzer),
            "mmx" => Some(Self::ModMatrix),
            // Modulation / Utility
            "rng" => Some(Self::RingMod),
            "efl" => Some(Self::EnvelopeFollower),
            "wtb" => Some(Self::WavetableOsc),
            "msg" => Some(Self::Mseg),
            "add" => Some(Self::AdditiveOsc),
            // Effects (continued)
            "bbd" => Some(Self::BbdDelay),
            "mds" => Some(Self::MidSide),
            "lmt" => Some(Self::Limiter),
            // Generative
            "euc" => Some(Self::Euclidean),
            "tur" => Some(Self::TuringMachine),
            "rgt" => Some(Self::RandomGates),
            // Physical modeling
            "kbp" => Some(Self::KeyboardPanner),
            "bdy" => Some(Self::BodyResonance),
            "mec" => Some(Self::MechanicalNoise),
            "grn" => Some(Self::GranularOsc),
            "cnv" => Some(Self::Convolver),
            "pvc" => Some(Self::PhaseVocoder),
            "kin" => Some(Self::KineticModulator),
            "smn" => Some(Self::SignalMonitor),
            "fsf" => Some(Self::FrequencyShifter),
            "vec" => Some(Self::VectorMixer),
            "las" => Some(Self::LaSynth),
            "ptr" => Some(Self::PitchTracker),
            "enc" => Some(Self::EnsembleChorus),
            "shr" => Some(Self::ShimmerReverb),
            "gfx" => Some(Self::GranularFx),
            "sbl" => Some(Self::SpectralBlur),
            "mdr" => Some(Self::ModalResonator),
            "rgr" => Some(Self::ReverseGateReverb),
            "frc" => Some(Self::FractalOsc),
            "sam" => Some(Self::Sampler),
            "ain" => Some(Self::AudioInput),
            // MusicDSP additions
            "ldr" => Some(Self::LadderFilter),
            "drf" => Some(Self::DriftGenerator),
            "cha" => Some(Self::ChaoticOsc),
            "fmt" => Some(Self::FormantFilter),
            "fog" => Some(Self::Fooglers),
            "btd" => Some(Self::BeatDetector),
            "pad" => Some(Self::PadSynth),
            "amf" => Some(Self::AmFormant),
            "teq" => Some(Self::TiltEq),
            "uvb" => Some(Self::Univibe),
            "cxo" => Some(Self::CrossoverSplitter),
            "vcd" => Some(Self::Vocoder),
            "vox" => Some(Self::VoiceSynth),
            "vtr" => Some(Self::VocalTract),
            "fof" => Some(Self::Fof),
            _ => None,
        }
    }

    /// Parse a module-type token supplied by a client in any of the forms it
    /// might reasonably use: the short prefix (`"osc"`), or the type name in
    /// `snake_case`, spaced, or lower-cased display form (`"oscillator"`,
    /// `"Oscillator"`, `"math oscillator"`, `"math_oscillator"`). All matching
    /// is case-insensitive. Returns `None` if the token is unrecognized.
    ///
    /// This is the forgiving inverse of [`prefix`](Self::prefix) /
    /// [`name`](Self::name): `list_module_types` advertises the prefix as the
    /// type key while tool descriptions and AI clients often pass the full
    /// name, so accepting both avoids a class of silent "unknown module type"
    /// failures.
    #[must_use]
    pub fn from_token(token: &str) -> Option<Self> {
        let lower = token.trim().to_ascii_lowercase();
        if let Some(mt) = Self::from_prefix(&lower) {
            return Some(mt);
        }
        // Normalize separators (spaces, hyphens, …) to '_' so the serde
        // `snake_case` names match spaced/display input as well.
        let snake: String = lower
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        use serde::de::IntoDeserializer;
        let de: serde::de::value::StrDeserializer<'_, serde::de::value::Error> =
            snake.as_str().into_deserializer();
        Self::deserialize(de).ok()
    }
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
    Waveshaper(WaveshaperParam),
    Oscilloscope(OscilloscopeParam),
    LevelMeter(LevelMeterParam),
    SpectrumAnalyzer(SpectrumAnalyzerParam),
    ModMatrix(ModMatrixParam),
    // Modulation / Utility
    RingMod(RingModParam),
    EnvelopeFollower(EnvelopeFollowerParam),
    WavetableOsc(WavetableParam),
    Mseg(MsegParam),
    AdditiveOsc(AdditiveParam),
    // Effects (continued)
    BbdDelay(BbdDelayParam),
    MidSide(MidSideParam),
    Limiter(LimiterParam),
    // Generative
    Euclidean(EuclideanParam),
    TuringMachine(TuringMachineParam),
    RandomGates(RandomGatesParam),
    // Physical modeling
    KeyboardPanner(KeyboardPannerParam),
    BodyResonance(BodyResonanceParam),
    MechanicalNoise(MechanicalNoiseParam),
    // Granular
    GranularOsc(GranularParam),
    // Spectral effects
    Convolver(ConvolverParam),
    PhaseVocoder(PhaseVocoderParam),
    // Kinetic modulation
    Kinetic(KineticParam),
    // Signal monitor
    SignalMonitor(SignalMonitorParam),
    // Spectral effects (continued)
    FrequencyShifter(FrequencyShifterParam),
    // Synthesis
    VectorMixer(VectorMixerParam),
    LaSynth(LaSynthParam),
    // Utility
    PitchTracker(PitchTrackerParam),
    // New effects (v0.189.0)
    EnsembleChorus(EnsembleChorusParam),
    ShimmerReverb(ShimmerReverbParam),
    GranularFx(GranularFxParam),
    SpectralBlur(SpectralBlurParam),
    ModalResonator(ModalResonatorParam),
    ReverseGateReverb(ReverseGateReverbParam),
    // Fractal synthesis
    FractalOsc(FractalOscParam),
    // Sampler
    Sampler(SamplerParam),
    // MusicDSP additions
    DriftGenerator(DriftGeneratorParam),
    ChaoticOsc(ChaoticOscParam),
    FormantFilter(FormantFilterParam),
    Fooglers(FooglersParam),
    BeatDetector(BeatDetectorParam),
    PadSynth(PadSynthParam),
    AmFormant(AmFormantParam),
    TiltEq(TiltEqParam),
    Univibe(UnivibeParam),
    Crossover(CrossoverParam),
    Vocoder(VocoderParam),
    // Synthesis (voice)
    VoiceSynth(VoiceSynthParam),
    VocalTract(VocalTractParam),
    Fof(FofParam),
}

impl Param {
    /// Ergonomic constructor for a sampler `SampleSelect` param from a raw
    /// sample id, collapsing the `Param::Sampler(SamplerParam::SampleSelect(
    /// SampleId(id)))` nesting at call sites.
    #[must_use]
    pub const fn sample_select(id: u64) -> Self {
        Self::Sampler(SamplerParam::sample_select(id))
    }

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
            (Self::Waveshaper(a), Self::Waveshaper(b)) => a.same_kind(b),
            (Self::Oscilloscope(a), Self::Oscilloscope(b)) => a.same_kind(b),
            (Self::LevelMeter(a), Self::LevelMeter(b)) => a.same_kind(b),
            (Self::SpectrumAnalyzer(a), Self::SpectrumAnalyzer(b)) => a.same_kind(b),
            (Self::ModMatrix(a), Self::ModMatrix(b)) => a.same_kind(b),
            // Modulation / Utility
            (Self::RingMod(a), Self::RingMod(b)) => a.same_kind(b),
            (Self::EnvelopeFollower(a), Self::EnvelopeFollower(b)) => a.same_kind(b),
            (Self::WavetableOsc(a), Self::WavetableOsc(b)) => a.same_kind(b),
            (Self::Mseg(a), Self::Mseg(b)) => a.same_kind(b),
            (Self::AdditiveOsc(a), Self::AdditiveOsc(b)) => a.same_kind(b),
            // Effects (continued)
            (Self::BbdDelay(a), Self::BbdDelay(b)) => a.same_kind(b),
            (Self::MidSide(a), Self::MidSide(b)) => a.same_kind(b),
            (Self::Limiter(a), Self::Limiter(b)) => a.same_kind(b),
            // Generative
            (Self::Euclidean(a), Self::Euclidean(b)) => a.same_kind(b),
            (Self::TuringMachine(a), Self::TuringMachine(b)) => a.same_kind(b),
            (Self::RandomGates(a), Self::RandomGates(b)) => a.same_kind(b),
            // Physical modeling
            (Self::KeyboardPanner(a), Self::KeyboardPanner(b)) => a.same_kind(b),
            (Self::BodyResonance(a), Self::BodyResonance(b)) => a.same_kind(b),
            (Self::MechanicalNoise(a), Self::MechanicalNoise(b)) => a.same_kind(b),
            (Self::GranularOsc(a), Self::GranularOsc(b)) => a.same_kind(b),
            (Self::Convolver(a), Self::Convolver(b)) => a.same_kind(b),
            (Self::PhaseVocoder(a), Self::PhaseVocoder(b)) => a.same_kind(b),
            (Self::Kinetic(a), Self::Kinetic(b)) => a.same_kind(b),
            (Self::SignalMonitor(a), Self::SignalMonitor(b)) => a.same_kind(b),
            (Self::FrequencyShifter(a), Self::FrequencyShifter(b)) => a.same_kind(b),
            (Self::VectorMixer(a), Self::VectorMixer(b)) => a.same_kind(b),
            (Self::LaSynth(a), Self::LaSynth(b)) => a.same_kind(b),
            (Self::PitchTracker(a), Self::PitchTracker(b)) => a.same_kind(b),
            (Self::EnsembleChorus(a), Self::EnsembleChorus(b)) => a.same_kind(b),
            (Self::ShimmerReverb(a), Self::ShimmerReverb(b)) => a.same_kind(b),
            (Self::GranularFx(a), Self::GranularFx(b)) => a.same_kind(b),
            (Self::SpectralBlur(a), Self::SpectralBlur(b)) => a.same_kind(b),
            (Self::ModalResonator(a), Self::ModalResonator(b)) => a.same_kind(b),
            (Self::ReverseGateReverb(a), Self::ReverseGateReverb(b)) => a.same_kind(b),
            (Self::FractalOsc(a), Self::FractalOsc(b)) => a.same_kind(b),
            (Self::Sampler(a), Self::Sampler(b)) => a.same_kind(b),
            // MusicDSP additions
            (Self::DriftGenerator(a), Self::DriftGenerator(b)) => a.same_kind(b),
            (Self::ChaoticOsc(a), Self::ChaoticOsc(b)) => a.same_kind(b),
            (Self::FormantFilter(a), Self::FormantFilter(b)) => a.same_kind(b),
            (Self::Fooglers(a), Self::Fooglers(b)) => a.same_kind(b),
            (Self::BeatDetector(a), Self::BeatDetector(b)) => a.same_kind(b),
            (Self::PadSynth(a), Self::PadSynth(b)) => a.same_kind(b),
            (Self::AmFormant(a), Self::AmFormant(b)) => a.same_kind(b),
            (Self::TiltEq(a), Self::TiltEq(b)) => a.same_kind(b),
            (Self::Univibe(a), Self::Univibe(b)) => a.same_kind(b),
            (Self::Crossover(a), Self::Crossover(b)) => a.same_kind(b),
            (Self::Vocoder(a), Self::Vocoder(b)) => a.same_kind(b),
            (Self::VoiceSynth(a), Self::VoiceSynth(b)) => a.same_kind(b),
            (Self::VocalTract(a), Self::VocalTract(b)) => a.same_kind(b),
            (Self::Fof(a), Self::Fof(b)) => a.same_kind(b),
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
            Self::Waveshaper(_) => ModuleType::Waveshaper,
            Self::Oscilloscope(_) => ModuleType::Oscilloscope,
            Self::LevelMeter(_) => ModuleType::LevelMeter,
            Self::SpectrumAnalyzer(_) => ModuleType::SpectrumAnalyzer,
            Self::ModMatrix(_) => ModuleType::ModMatrix,
            // Modulation / Utility
            Self::RingMod(_) => ModuleType::RingMod,
            Self::EnvelopeFollower(_) => ModuleType::EnvelopeFollower,
            Self::WavetableOsc(_) => ModuleType::WavetableOsc,
            Self::Mseg(_) => ModuleType::Mseg,
            Self::AdditiveOsc(_) => ModuleType::AdditiveOsc,
            // Effects (continued)
            Self::BbdDelay(_) => ModuleType::BbdDelay,
            Self::MidSide(_) => ModuleType::MidSide,
            Self::Limiter(_) => ModuleType::Limiter,
            // Generative
            Self::Euclidean(_) => ModuleType::Euclidean,
            Self::TuringMachine(_) => ModuleType::TuringMachine,
            Self::RandomGates(_) => ModuleType::RandomGates,
            // Physical modeling
            Self::KeyboardPanner(_) => ModuleType::KeyboardPanner,
            Self::BodyResonance(_) => ModuleType::BodyResonance,
            Self::MechanicalNoise(_) => ModuleType::MechanicalNoise,
            Self::GranularOsc(_) => ModuleType::GranularOsc,
            Self::Convolver(_) => ModuleType::Convolver,
            Self::PhaseVocoder(_) => ModuleType::PhaseVocoder,
            Self::Kinetic(_) => ModuleType::KineticModulator,
            Self::SignalMonitor(_) => ModuleType::SignalMonitor,
            Self::FrequencyShifter(_) => ModuleType::FrequencyShifter,
            Self::VectorMixer(_) => ModuleType::VectorMixer,
            Self::LaSynth(_) => ModuleType::LaSynth,
            Self::PitchTracker(_) => ModuleType::PitchTracker,
            Self::EnsembleChorus(_) => ModuleType::EnsembleChorus,
            Self::ShimmerReverb(_) => ModuleType::ShimmerReverb,
            Self::GranularFx(_) => ModuleType::GranularFx,
            Self::SpectralBlur(_) => ModuleType::SpectralBlur,
            Self::ModalResonator(_) => ModuleType::ModalResonator,
            Self::ReverseGateReverb(_) => ModuleType::ReverseGateReverb,
            Self::FractalOsc(_) => ModuleType::FractalOsc,
            Self::Sampler(_) => ModuleType::Sampler,
            // MusicDSP additions
            Self::DriftGenerator(_) => ModuleType::DriftGenerator,
            Self::ChaoticOsc(_) => ModuleType::ChaoticOsc,
            Self::FormantFilter(_) => ModuleType::FormantFilter,
            Self::Fooglers(_) => ModuleType::Fooglers,
            Self::BeatDetector(_) => ModuleType::BeatDetector,
            Self::PadSynth(_) => ModuleType::PadSynth,
            Self::AmFormant(_) => ModuleType::AmFormant,
            Self::TiltEq(_) => ModuleType::TiltEq,
            Self::Univibe(_) => ModuleType::Univibe,
            Self::Crossover(_) => ModuleType::CrossoverSplitter,
            Self::Vocoder(_) => ModuleType::Vocoder,
            Self::VoiceSynth(_) => ModuleType::VoiceSynth,
            Self::VocalTract(_) => ModuleType::VocalTract,
            Self::Fof(_) => ModuleType::Fof,
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
            Self::Waveshaper(p) => p.name(),
            Self::Oscilloscope(p) => p.name(),
            Self::LevelMeter(p) => p.name(),
            Self::SpectrumAnalyzer(p) => p.name(),
            Self::ModMatrix(p) => p.name(),
            // Modulation / Utility
            Self::RingMod(p) => p.name(),
            Self::EnvelopeFollower(p) => p.name(),
            Self::WavetableOsc(p) => p.name(),
            Self::Mseg(p) => p.name(),
            Self::AdditiveOsc(p) => p.name(),
            // Effects (continued)
            Self::BbdDelay(p) => p.name(),
            Self::MidSide(p) => p.name(),
            Self::Limiter(p) => p.name(),
            // Generative
            Self::Euclidean(p) => p.name(),
            Self::TuringMachine(p) => p.name(),
            Self::RandomGates(p) => p.name(),
            // Physical modeling
            Self::KeyboardPanner(p) => p.name(),
            Self::BodyResonance(p) => p.name(),
            Self::MechanicalNoise(p) => p.name(),
            Self::GranularOsc(p) => p.name(),
            Self::Convolver(p) => p.name(),
            Self::PhaseVocoder(p) => p.name(),
            Self::Kinetic(p) => p.name(),
            Self::SignalMonitor(p) => p.name(),
            Self::FrequencyShifter(p) => p.name(),
            Self::VectorMixer(p) => p.name(),
            Self::LaSynth(p) => p.name(),
            Self::PitchTracker(p) => p.name(),
            Self::EnsembleChorus(p) => p.name(),
            Self::ShimmerReverb(p) => p.name(),
            Self::GranularFx(p) => p.name(),
            Self::SpectralBlur(p) => p.name(),
            Self::ModalResonator(p) => p.name(),
            Self::ReverseGateReverb(p) => p.name(),
            Self::FractalOsc(p) => p.name(),
            Self::Sampler(p) => p.name(),
            // MusicDSP additions
            Self::DriftGenerator(p) => p.name(),
            Self::ChaoticOsc(p) => p.name(),
            Self::FormantFilter(p) => p.name(),
            Self::Fooglers(p) => p.name(),
            Self::BeatDetector(p) => p.name(),
            Self::PadSynth(p) => p.name(),
            Self::AmFormant(p) => p.name(),
            Self::TiltEq(p) => p.name(),
            Self::Univibe(p) => p.name(),
            Self::Crossover(p) => p.name(),
            Self::Vocoder(p) => p.name(),
            Self::VoiceSynth(p) => p.name(),
            Self::VocalTract(p) => p.name(),
            Self::Fof(p) => p.name(),
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
            Self::Waveshaper(p) => p.as_f32(),
            Self::Oscilloscope(p) => p.as_f32(),
            Self::LevelMeter(p) => p.as_f32(),
            Self::SpectrumAnalyzer(p) => p.as_f32(),
            Self::ModMatrix(p) => p.as_f32(),
            // Modulation / Utility
            Self::RingMod(p) => p.as_f32(),
            Self::EnvelopeFollower(p) => p.as_f32(),
            Self::WavetableOsc(p) => p.as_f32(),
            Self::Mseg(p) => p.as_f32(),
            Self::AdditiveOsc(p) => p.as_f32(),
            // Effects (continued)
            Self::BbdDelay(p) => p.as_f32(),
            Self::MidSide(p) => p.as_f32(),
            Self::Limiter(p) => p.as_f32(),
            // Generative
            Self::Euclidean(p) => p.as_f32(),
            Self::TuringMachine(p) => p.as_f32(),
            Self::RandomGates(p) => p.as_f32(),
            // Physical modeling
            Self::KeyboardPanner(p) => p.as_f32(),
            Self::BodyResonance(p) => p.as_f32(),
            Self::MechanicalNoise(p) => p.as_f32(),
            Self::GranularOsc(p) => p.as_f32(),
            Self::Convolver(p) => p.as_f32(),
            Self::PhaseVocoder(p) => p.as_f32(),
            Self::Kinetic(p) => p.as_f32(),
            Self::SignalMonitor(p) => p.as_f32(),
            Self::FrequencyShifter(p) => p.as_f32(),
            Self::VectorMixer(p) => p.as_f32(),
            Self::LaSynth(p) => p.as_f32(),
            Self::PitchTracker(p) => p.as_f32(),
            Self::EnsembleChorus(p) => p.as_f32(),
            Self::ShimmerReverb(p) => p.as_f32(),
            Self::GranularFx(p) => p.as_f32(),
            Self::SpectralBlur(p) => p.as_f32(),
            Self::ModalResonator(p) => p.as_f32(),
            Self::ReverseGateReverb(p) => p.as_f32(),
            Self::FractalOsc(p) => p.as_f32(),
            Self::Sampler(p) => p.as_f32(),
            // MusicDSP additions
            Self::DriftGenerator(p) => p.as_f32(),
            Self::ChaoticOsc(p) => p.as_f32(),
            Self::FormantFilter(p) => p.as_f32(),
            Self::Fooglers(p) => p.as_f32(),
            Self::BeatDetector(p) => p.as_f32(),
            Self::PadSynth(p) => p.as_f32(),
            Self::AmFormant(p) => p.as_f32(),
            Self::TiltEq(p) => p.as_f32(),
            Self::Univibe(p) => p.as_f32(),
            Self::Crossover(p) => p.as_f32(),
            Self::Vocoder(p) => p.as_f32(),
            Self::VoiceSynth(p) => p.as_f32(),
            Self::VocalTract(p) => p.as_f32(),
            Self::Fof(p) => p.as_f32(),
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
            Self::Waveshaper(p) => Self::Waveshaper(p.with_f32(value)),
            Self::Oscilloscope(p) => Self::Oscilloscope(p.with_f32(value)),
            Self::LevelMeter(p) => Self::LevelMeter(p.with_f32(value)),
            Self::SpectrumAnalyzer(p) => Self::SpectrumAnalyzer(p.with_f32(value)),
            Self::ModMatrix(p) => Self::ModMatrix(p.with_f32(value)),
            // Modulation / Utility
            Self::RingMod(p) => Self::RingMod(p.with_f32(value)),
            Self::EnvelopeFollower(p) => Self::EnvelopeFollower(p.with_f32(value)),
            Self::WavetableOsc(p) => Self::WavetableOsc(p.with_f32(value)),
            Self::Mseg(p) => Self::Mseg(p.with_f32(value)),
            Self::AdditiveOsc(p) => Self::AdditiveOsc(p.with_f32(value)),
            // Effects (continued)
            Self::BbdDelay(p) => Self::BbdDelay(p.with_f32(value)),
            Self::MidSide(p) => Self::MidSide(p.with_f32(value)),
            Self::Limiter(p) => Self::Limiter(p.with_f32(value)),
            // Generative
            Self::Euclidean(p) => Self::Euclidean(p.with_f32(value)),
            Self::TuringMachine(p) => Self::TuringMachine(p.with_f32(value)),
            Self::RandomGates(p) => Self::RandomGates(p.with_f32(value)),
            // Physical modeling
            Self::KeyboardPanner(p) => Self::KeyboardPanner(p.with_f32(value)),
            Self::BodyResonance(p) => Self::BodyResonance(p.with_f32(value)),
            Self::MechanicalNoise(p) => Self::MechanicalNoise(p.with_f32(value)),
            Self::GranularOsc(p) => Self::GranularOsc(p.with_f32(value)),
            Self::Convolver(p) => Self::Convolver(p.with_f32(value)),
            Self::PhaseVocoder(p) => Self::PhaseVocoder(p.with_f32(value)),
            Self::Kinetic(p) => Self::Kinetic(p.with_f32(value)),
            Self::SignalMonitor(p) => Self::SignalMonitor(p.with_f32(value)),
            Self::FrequencyShifter(p) => Self::FrequencyShifter(p.with_f32(value)),
            Self::VectorMixer(p) => Self::VectorMixer(p.with_f32(value)),
            Self::LaSynth(p) => Self::LaSynth(p.with_f32(value)),
            Self::PitchTracker(p) => Self::PitchTracker(p.with_f32(value)),
            Self::EnsembleChorus(p) => Self::EnsembleChorus(p.with_f32(value)),
            Self::ShimmerReverb(p) => Self::ShimmerReverb(p.with_f32(value)),
            Self::GranularFx(p) => Self::GranularFx(p.with_f32(value)),
            Self::SpectralBlur(p) => Self::SpectralBlur(p.with_f32(value)),
            Self::ModalResonator(p) => Self::ModalResonator(p.with_f32(value)),
            Self::ReverseGateReverb(p) => Self::ReverseGateReverb(p.with_f32(value)),
            Self::FractalOsc(p) => Self::FractalOsc(p.with_f32(value)),
            Self::Sampler(p) => Self::Sampler(p.with_f32(value)),
            // MusicDSP additions
            Self::DriftGenerator(p) => Self::DriftGenerator(p.with_f32(value)),
            Self::ChaoticOsc(p) => Self::ChaoticOsc(p.with_f32(value)),
            Self::FormantFilter(p) => Self::FormantFilter(p.with_f32(value)),
            Self::Fooglers(p) => Self::Fooglers(p.with_f32(value)),
            Self::BeatDetector(p) => Self::BeatDetector(p.with_f32(value)),
            Self::PadSynth(p) => Self::PadSynth(p.with_f32(value)),
            Self::AmFormant(p) => Self::AmFormant(p.with_f32(value)),
            Self::TiltEq(p) => Self::TiltEq(p.with_f32(value)),
            Self::Univibe(p) => Self::Univibe(p.with_f32(value)),
            Self::Crossover(p) => Self::Crossover(p.with_f32(value)),
            Self::Vocoder(p) => Self::Vocoder(p.with_f32(value)),
            Self::VoiceSynth(p) => Self::VoiceSynth(p.with_f32(value)),
            Self::VocalTract(p) => Self::VocalTract(p.with_f32(value)),
            Self::Fof(p) => Self::Fof(p.with_f32(value)),
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
    fn from_token_accepts_prefix_snake_and_display() {
        // Short prefix (what list_module_types advertises as the type key).
        assert_eq!(ModuleType::from_token("osc"), Some(ModuleType::Oscillator));
        assert_eq!(ModuleType::from_token("flt"), Some(ModuleType::Filter));
        // snake_case serde name.
        assert_eq!(
            ModuleType::from_token("oscillator"),
            Some(ModuleType::Oscillator)
        );
        assert_eq!(
            ModuleType::from_token("math_oscillator"),
            Some(ModuleType::MathOscillator)
        );
        // Case-insensitive + spaced/display forms.
        assert_eq!(
            ModuleType::from_token("Oscillator"),
            Some(ModuleType::Oscillator)
        );
        assert_eq!(
            ModuleType::from_token("Math Oscillator"),
            Some(ModuleType::MathOscillator)
        );
        assert_eq!(ModuleType::from_token("  LFO  "), Some(ModuleType::Lfo));
        // Unknown tokens.
        assert_eq!(ModuleType::from_token("definitely_not_a_module"), None);
        assert_eq!(ModuleType::from_token(""), None);
    }

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
        assert!(ModuleType::Waveshaper.is_effect());

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
