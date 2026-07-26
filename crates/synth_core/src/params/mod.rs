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
mod kind_impls;
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
mod scalar_impls;
mod script;
mod sid_oscillator;
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
    TransientShaperParam, UnivibeParam, VocoderParam,
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
    AlgoParamInfo, AntiAliasMode, FmMode, MathAlgo, MathOscillatorParam, OscillatorParam, Waveform,
};
pub use padsynth::PadSynthParam;
pub use phase_vocoder::{FftSizeOption, PhaseVocoderParam};
pub use physical::{
    BodyResonanceParam, KeyboardPannerParam, MechanicalNoiseParam, MechanicalNoiseType,
    SpatialPannerParam,
};
pub use pitch_tracker::PitchTrackerParam;
pub use ring_mod::RingModParam;
pub use sampler::{PlayDirection, SampleId, SamplerParam, SamplerPlayMode};
pub use script::ScriptParam;
pub use sid_oscillator::{
    SID_FREQ_REG_MAX, SID_PW_REG_MAX, SID_SEQ_STEPS, SidClock, SidModel, SidNoiseSeed,
    SidOscillatorParam, SidQuality,
};
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
    strum::EnumIter,
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
    /// Differential-envelope transient designer: independent attack/sustain
    /// gain shaping without altering the underlying amplitude.
    TransientShaper,
    // Synthesis (voice)
    VoiceSynth,
    VocalTract,
    Fof,
    // Scripting (per-voice control-signal generator)
    Script,
    /// Per-voice audio-rate scripted DSP (YAMS, one eval per sample): waveshaper,
    /// bitcrusher, ring-mod, custom IIR/filter. Audio in/out ports.
    AudioScript,
    /// MOS 6581/8580 (SID) waveform generator: combinable waveforms, noise
    /// LFSR, ring/hard-sync, TEST, per-model DAC.
    SidOscillator,
    /// Per-voice binaural spatial panner: ISM early reflections + ITD/ILD
    /// direct path positioning a mono voice in a fixed virtual room.
    SpatialPanner,
}

impl ModuleType {
    /// Every module type, in declaration order. Self-enumerating via
    /// `EnumIter`, so module catalogs (MCP discovery, the GUI "Add module"
    /// menu) are generated from the enum and can never drift from it — a new
    /// variant is automatically included.
    pub fn all() -> impl Iterator<Item = Self> {
        use strum::IntoEnumIterator;
        Self::iter()
    }

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
                | Self::SpatialPanner
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
                // Scripting (per-voice control-signal generator)
                | Self::Script
                // Audio-rate scripted DSP (per-voice)
                | Self::AudioScript
                // Chip-accurate SID waveform generator
                | Self::SidOscillator
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
                | Self::TransientShaper
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

    /// Whether this module's YAMS script compiles in the **audio-rate** dialect
    /// (one eval per sample: `in`/`in_l`/`in_r`, `first_sample`,
    /// `out.left`/`out.right`) rather than the control-rate dialect used by the
    /// Mod Matrix and the control `Script` module.
    ///
    /// Single source of truth for the dialect decision shared by every script
    /// install/compile site (`session::set_mod_script`, the offline-render
    /// `audio::replay_module_scripts`); a new audio-rate scripted module type is
    /// added here, not in each caller.
    #[inline]
    #[must_use]
    pub fn script_is_audio_rate(&self) -> bool {
        matches!(self, Self::AudioScript)
    }

    /// Whether this module's YAMS script compiles in the **control-ports** dialect
    /// (the `Script` module): numbered CV ports `in1..in4` (read) and `out1..out4`
    /// (written), one program. Distinct from the Mod Matrix's `scr` scripts (no
    /// ports, single `out`) and the audio-rate `AudioScript`. Single source of
    /// truth for the dialect decision, shared by every script install/compile site
    /// (`session::set_mod_script`, the offline renderer, the GUI editor).
    #[inline]
    #[must_use]
    pub fn script_uses_control_ports(&self) -> bool {
        matches!(self, Self::Script)
    }

    // ========================================================================
    // DISPLAY AND CONVERSION METHODS
    // ========================================================================

    /// The module type's display name.
    ///
    /// This must match the name in the module's own `ModuleDescriptor` — the two
    /// live in different crates (a descriptor is built in `synth_modules`, which
    /// depends on this one), so equality is enforced by the
    /// `module_type_names_match_their_descriptors` test rather than by
    /// construction. Where they once disagreed, the descriptor's shorter
    /// synth-idiomatic name won: an envelope is an "ADSR", an amplifier a "VCA".
    pub fn name(&self) -> &'static str {
        match self {
            Self::Oscillator => "Oscillator",
            Self::MathOscillator => "Math Oscillator",
            Self::SubOscillator => "Sub Osc",
            Self::Noise => "Noise",
            Self::Filter => "Filter",
            Self::Envelope => "ADSR",
            Self::Lfo => "LFO",
            Self::Amplifier => "VCA",
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
            Self::Oscilloscope => "Scope",
            Self::LevelMeter => "Meter",
            Self::SpectrumAnalyzer => "Spectrum",
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
            Self::SignalMonitor => "Sig Mon",
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
            Self::TransientShaper => "Transient Shaper",
            Self::VoiceSynth => "Voice Synth",
            Self::VocalTract => "Vocal Tract",
            Self::Fof => "FOF",
            Self::Script => "Script",
            Self::AudioScript => "Audio Script",
            Self::SidOscillator => "SID Oscillator",
            Self::SpatialPanner => "Spatial Panner",
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
            Self::TransientShaper => "tsh",
            Self::VoiceSynth => "vox",
            Self::VocalTract => "vtr",
            Self::Fof => "fof",
            Self::Script => "scr",
            Self::AudioScript => "asc",
            Self::SidOscillator => "sid",
            Self::SpatialPanner => "spp",
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
            "tsh" => Some(Self::TransientShaper),
            "vox" => Some(Self::VoiceSynth),
            "vtr" => Some(Self::VocalTract),
            "fof" => Some(Self::Fof),
            "scr" => Some(Self::Script),
            "asc" => Some(Self::AudioScript),
            "sid" => Some(Self::SidOscillator),
            "spp" => Some(Self::SpatialPanner),
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
    TransientShaper(TransientShaperParam),
    // Synthesis (voice)
    VoiceSynth(VoiceSynthParam),
    VocalTract(VocalTractParam),
    Fof(FofParam),
    // Chip-accurate SID waveform generator
    SidOscillator(SidOscillatorParam),
    SpatialPanner(SpatialPannerParam),
    // Script modules' user-declared knobs (dynamic, name-keyed via an interned
    // PortName so `Param` stays `Copy`). Shared by `Script` and `AudioScript`.
    Script(ScriptParam),
}

impl Param {
    /// Ergonomic constructor for a sampler `SampleSelect` param from a raw
    /// sample id, collapsing the `Param::Sampler(SamplerParam::SampleSelect(
    /// SampleId(id)))` nesting at call sites.
    #[must_use]
    pub const fn sample_select(id: u64) -> Self {
        Self::Sampler(SamplerParam::sample_select(id))
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
            Self::TransientShaper(_) => ModuleType::TransientShaper,
            Self::VoiceSynth(_) => ModuleType::VoiceSynth,
            Self::VocalTract(_) => ModuleType::VocalTract,
            Self::Fof(_) => ModuleType::Fof,
            Self::SidOscillator(_) => ModuleType::SidOscillator,
            Self::SpatialPanner(_) => ModuleType::SpatialPanner,
            // A script knob is shared by `Script` and `AudioScript`; the concrete
            // module is always resolved by `ModuleId`, never by this discriminant,
            // so it reports the primary `Script` type.
            Self::Script(_) => ModuleType::Script,
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
    use crate::module_traits::ModuleParam;
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
