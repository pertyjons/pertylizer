//! Core types, traits, and audio abstractions for Pertylizer.
//!
//! This crate provides the foundational types used throughout the synthesizer:
//! - Type-safe audio domain types (Hertz, Gain, etc.)
//! - Module traits (PolyModule, AudioEffect, Describable)
//! - Audio processing traits (AudioProcessor, AudioBackend)

// Crate-level clippy allows for synth-appropriate exceptions
#![allow(clippy::must_use_candidate)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::similar_names)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::suboptimal_flops)]
#![allow(clippy::float_cmp)]

pub mod audio;
pub mod display;
pub mod hash;
pub mod module_traits;
pub mod params;
pub mod script;
pub mod tuning;
pub mod types;

// Re-export all types at crate root for convenience
pub use audio::{
    AudioBackend, AudioCallbackContext, AudioError, AudioHost, AudioHostTrait, AudioProcessor,
    AudioResult, AudioStream, BufferSize, ChannelCount, DeviceInfo, DeviceType, MAX_BLOCK_SIZE,
    StreamConfig, StreamInfo,
};
pub use display::DisplayName;
pub use module_traits::{
    AudioBuffer, AudioEffect, ChoiceOption, Describable, InputPorts, InputReader, ModuleCategory,
    ModuleDescriptor, ModuleParam, ModuleTypeId, ModuleWidth, ParamKind, ParamModOffsets,
    ParamValueError, ParameterDescriptor, ParameterUnit, PolyModule, PortDescriptor, PortDirection,
    PortType, PortValueDomain, PortValueRange, ProcessContext, ResponseCurve, ScalarParam,
    VisualizationSink, VoicePitch, WidgetHint,
};
pub use params::{
    AdditiveParam, AlgoParamInfo, AmFormantParam, AmplifierParam, AntiAliasMode, AudioPort,
    BbdDelayParam, BeatDetectorParam, BodyResonanceParam, ChaoticOscParam, ChaoticSystem,
    ChorusParam, CompressorParam, ControlPort, ConvolverParam, CrossoverParam, DelayMode,
    DelayParam, DestAddr, DistortionMode, DistortionParam, DriftGeneratorParam, EasingCurve,
    EnsembleChorusParam, EnvelopeFollowerParam, EnvelopeParam, EqParam, EuclideanParam,
    FftSizeOption, FilterMode, FilterModel, FilterParam, FlangerParam, FmMode, FofParam,
    FooglersParam, FormantFilterParam, FractalOscParam, FrequencyShifterParam, GrainSource,
    GrainWindow, GranularFxParam, GranularParam, ImpulseResponse, KeyboardPannerParam,
    KineticLoopMode, KineticParam, LaSynthParam, LevelMeterParam, LfoParam, LfoWaveform,
    LimiterParam, MAX_MOD_MATRIX_SLOTS, MacroSource, MathAlgo, MathOscillatorParam,
    MechanicalNoiseParam, MechanicalNoiseType, MidSideParam, MixerParam, ModDestination,
    ModMatrixGridSize, ModMatrixParam, ModRouting, ModSource, ModalResonatorParam, ModuleType,
    MsegParam, NoiseParam, NoiseType, OscillatorParam, OscilloscopeParam, PadSynthParam, Param,
    PhaseVocoderParam, PhaserParam, PitchTrackerParam, PlayDirection, Port, RandomGatesParam,
    ReverbParam, ReverseGateMode, ReverseGateReverbParam, ReverseGateTrigger, RingModParam,
    SID_FREQ_REG_MAX, SID_PW_REG_MAX, SID_SEQ_STEPS, SampleId, SamplerParam, SamplerPlayMode,
    ScriptParam, ShimmerReverbParam, SidClock, SidModel, SidNoiseSeed, SidOscillatorParam,
    SidQuality, SignalMonitorParam, SpatialPannerParam, SpectralBlurParam, SpectrumAnalyzerParam,
    SrcAddr, SubOscOctave, SubOscParam, SubOscWaveform, TiltEqParam, TransientShaperParam,
    TuringMachineParam, TuringScale, UnivibeParam, VectorMixerParam, VocalTractParam, VocoderParam,
    VoiceSynthParam, Waveform, WaveshaperCurve, WaveshaperParam, WavetableParam, WavetableSelect,
    easing_acceleration, easing_position, easing_velocity,
};
pub use types::{
    AllpassState, Amplitude, BeatDivision, BeatPosition, BipolarDelta, BipolarValue, BitDepth,
    BlockSize, Bpm, BufferIndex, BypassState, CcNumber, Cents, Clampable, ClipMode,
    ContentRevision, CpuUsage, DecayTrim, Decibels, DenormalGuard, EnableState, FilterState,
    FrameCount, FreezeState, FrequencyBand, Gain, Hertz, InstrumentId, Interpolate, LimitMode,
    Meters, MetersPerSecond, MidiCcNumber, MidiChannel, MidiNote, Milliseconds, MuteState,
    NoiseState, NormalizedDelta, NormalizedValue, NoteReleaseState, Octaves, OnePoleSmooth,
    PatternIndex, Phase, Polarity, PortName, Position3, PulseWidth, Ratio, RetriggerMode,
    SampleCount, SampleOffset, SamplePosition, SampleRate, Seconds, Semitones, SoloState,
    StepCount, StereoBalance, StereoFrameIter, StereoLevels, StereoSample, SyncMode,
    TempoSyncState, TimeScale, ValueRange, Velocity, VoiceCount, magnitude_to_normalized_db,
};

/// Convenience re-exports for code that works with parameters generically.
///
/// `use synth_core::prelude::*;` brings the parameter traits and the value-kind
/// classifier into scope so generic `T: ModuleParam` / `T: ScalarParam` code reads
/// cleanly. (Everything here is also reachable directly at the crate root.)
pub mod prelude {
    pub use crate::module_traits::{
        ModuleParam, ParamKind, ParameterUnit, ResponseCurve, ScalarParam,
    };
    pub use crate::params::Param;
}
